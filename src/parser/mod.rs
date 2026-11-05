use crate::citation::{CitationManager, CitationMode, parse_citation_args};
use crate::commands::{apply_accent, special_char_to_string, CommandRegistry};
use crate::error::Result;
use crate::include_system::IncludeSystem;
use crate::lexer::Lexer;
use crate::lexer::Token;
use crate::lexer_extended::{get_accent_type, get_special_char};
use crate::macro_processor::MacroProcessor;
use crate::state::{LabelType, MacroDef, ParserState};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

mod algorithms;
mod figures;
mod glossary;
mod knowledge;
mod math;
mod siunitx;
mod tables;
mod tex;

/// Baseline budget of macro expansions per document, guarding against
/// self-referential macro definitions looping forever. The effective budget
/// also grows with document size (see `macro_budget`).
const MAX_MACRO_EXPANSIONS: usize = 100_000;

/// Hard cap on pending tokens; an expansion that would grow the stream past
/// this is refused (exponentially self-replicating macros).
const MAX_PENDING_TOKENS: usize = 4_000_000;

/// Baseline for the total bytes all macro expansions may produce; the
/// effective budget adds a multiple of the source size. Guards against
/// macros whose arguments grow on every recursion (`\a{#1#1}`).
const BASE_EXPANSION_BYTES: usize = 64 * 1024 * 1024;


/// Maximum recursion depth of block/inline parsing. Real documents nest a
/// few dozen levels at most; beyond this the input is pathological and is
/// flattened instead of recursed into, so the parser never overflows its stack.
const MAX_NESTING_DEPTH: usize = 200;

/// Upper bound on table columns (a column spec like `*{999999999}{c}` must
/// not allocate unboundedly).
const MAX_TABLE_COLUMNS: usize = 1_000;

/// Lists nested deeper than this are flattened into their parent.
const MAX_LIST_NESTING: usize = 12;

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    // Document structure
    Section { level: u8, title: Vec<Inline>, label: Option<String> },
    Paragraph(Vec<Inline>),
    
    // Lists
    BulletList(Vec<Vec<Block>>),
    OrderedList { start: usize, items: Vec<Vec<Block>> },
    DescriptionList(Vec<(Vec<Inline>, Vec<Block>)>),
    
    // Environments
    Quote(Vec<Block>),
    CodeBlock { language: Option<String>, content: String },
    Verbatim(String),
    
    // Math
    DisplayMath(String),
    
    // Tables
    Table {
        caption: Option<Vec<Inline>>,
        alignments: Vec<Alignment>,
        headers: Vec<Vec<Block>>,
        rows: Vec<Vec<Vec<Block>>>,
    },
    
    // Figures
    Figure {
        caption: Option<Vec<Inline>>,
        path: String,
        label: Option<String>,
    },

    Composite(Vec<Block>),
    
    // Special blocks
    TheoremLike {
        env_type: String,
        display_name: String,
        number: Option<String>,
        label: Option<String>,
        title: Option<Vec<Inline>>,
        content: Vec<Block>,
    },
    RawBlock(String),
    HorizontalRule,
    Null,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text(String),
    Space,
    SoftBreak,
    LineBreak,
    
    // Formatting
    Emph(Vec<Inline>),
    Strong(Vec<Inline>),
    Strikeout(Vec<Inline>),
    Underline(Vec<Inline>),
    Superscript(Vec<Inline>),
    Subscript(Vec<Inline>),
    SmallCaps(Vec<Inline>),
    
    // Code
    Code(String),
    
    // Math
    InlineMath(String),
    
    // Links and references
    Link { text: Vec<Inline>, url: String, title: Option<String> },
    Image { alt: Vec<Inline>, url: String, title: Option<String> },
    /// A citation. `content` is empty until the post-parse resolution pass
    /// renders it (after .bib files are loaded and the style is known).
    Cite {
        citations: Vec<String>,
        mode: CitationMode,
        prenote: Option<String>,
        postnote: Option<String>,
        content: Vec<Inline>,
    },
    Ref { kind: String, label: String },
    
    // Special
    RawInline(String),
    Note(Vec<Block>),
    Span { attrs: HashMap<String, String>, content: Vec<Inline> },
    Quoted { quote_type: QuoteType, content: Vec<Inline> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum QuoteType {
    SingleQuote,
    DoubleQuote,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Alignment {
    AlignLeft,
    AlignRight,
    AlignCenter,
    AlignDefault,
}

#[derive(Debug)]
pub struct Document {
    pub metadata: HashMap<String, String>,
    pub blocks: Vec<Block>,
    pub footnotes: HashMap<usize, String>,
    pub bibliography: Option<String>,
    /// Conversion diagnostics: constructs that could not be converted
    /// faithfully (unknown commands, unresolved references, ...).
    pub warnings: Vec<String>,
}

pub struct Parser {
    /// Remaining tokens, stored *reversed* (the current token is the last
    /// element) so that advancing is a pop and splicing a macro expansion in
    /// front of the cursor is O(expansion length) rather than O(document).
    tokens: Vec<Token>,
    /// Total tokens consumed so far (monotonic; used to guarantee progress).
    consumed: u64,
    /// Nesting depth of parse_block/parse_inline recursion; bounded by
    /// MAX_NESTING_DEPTH so pathological input cannot overflow the stack.
    depth: usize,
    metadata: HashMap<String, String>,
    state: ParserState,
    citation_manager: CitationManager,
    macro_processor: MacroProcessor,
    include_system: IncludeSystem,
    command_registry: CommandRegistry,
    base_path: Option<std::path::PathBuf>,
    macro_expansions: usize,
    macro_budget: usize,
    /// Start number for the next enumerate list (from \setcounter{enumi}).
    pending_enum_start: Option<usize>,
    /// The definition being parsed uses TeX parameter text (\def family).
    macro_def_tex_style: bool,
    /// Current itemize/enumerate nesting depth.
    list_depth: usize,
    /// Before `\begin{document}` (only when the source has one).
    in_preamble: bool,
    /// Expanding macros inside math (`\ifmmode` is true).
    in_math_expansion: bool,
    /// `\end{document}` was reached (possibly inside an included file).
    document_ended: bool,
    /// Preamble state to restore when each loaded package ends.
    package_preamble_restore: Vec<bool>,
    /// Open TeX conditionals whose taken branch is being parsed.
    cond_depth: usize,
    /// Macro expansions whose tokens are still pending: (name, stack length
    /// below the expansion). Used to detect runaway recursion.
    active_expansions: Vec<(String, usize)>,
    /// Expansions performed per macro.
    expansion_counts: HashMap<String, usize>,
    /// How often each command name occurs in the source (scales budgets).
    source_command_counts: HashMap<String, usize>,
    /// Number of tokens in the original source.
    source_token_count: usize,
    /// Total bytes produced by macro expansions so far.
    expanded_bytes: usize,
    /// Acronyms and glossary entries by key.
    glossary: HashMap<String, glossary::GlossaryEntry>,
    /// Numbered algorithm floats so far.
    algorithm_counter: usize,
    /// A \label found inside the caption just parsed.
    pending_caption_label: Option<String>,
    /// Macros whose expansion was refused (warned once).
    refused_macros: HashSet<String>,
    /// Footnote bodies kept as inlines until end of parse so forward
    /// references inside them can be resolved.
    footnote_inlines: HashMap<usize, Vec<Inline>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MacroDefinitionMode {
    New,
    Renew,
    Provide,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        use crate::state::CitationStyle;
        let macro_budget = MAX_MACRO_EXPANSIONS.saturating_add(tokens.len().saturating_mul(4));
        let source_token_count = tokens.len();
        let mut source_command_counts: HashMap<String, usize> = HashMap::new();
        for token in &tokens {
            if let Token::Command(name) = token {
                *source_command_counts.entry(name.clone()).or_insert(0) += 1;
            }
        }

        Self {
            tokens: reverse_tokens(tokens),
            depth: 0,
            consumed: 0,
            metadata: HashMap::new(),
            state: ParserState::new(),
            citation_manager: CitationManager::new(CitationStyle::Numeric),
            macro_processor: MacroProcessor::new(),
            include_system: IncludeSystem::new(),
            command_registry: CommandRegistry::new(),
            base_path: None,
            macro_expansions: 0,
            macro_budget,
            pending_enum_start: None,
            macro_def_tex_style: false,
            list_depth: 0,
            in_preamble: false,
            in_math_expansion: false,
            document_ended: false,
            package_preamble_restore: Vec::new(),
            cond_depth: 0,
            active_expansions: Vec::new(),
            expansion_counts: HashMap::new(),
            source_command_counts,
            source_token_count,
            expanded_bytes: 0,
            glossary: HashMap::new(),
            algorithm_counter: 0,
            pending_caption_label: None,
            refused_macros: HashSet::new(),
            footnote_inlines: HashMap::new(),
        }
    }
    
    pub fn with_base_path(mut self, path: std::path::PathBuf) -> Self {
        self.base_path = Some(path);
        self
    }

    pub fn parse(&mut self) -> Result<Document> {
        self.in_preamble = self
            .tokens
            .iter()
            .any(|t| matches!(t, Token::BeginEnvironment(env) if env == "document"));
        let mut blocks = self.parse_blocks()?;

        // Bibliography must load before the resolution pass: author-year
        // citation rendering needs the .bib entries.
        self.load_bibliography_files();

        // Second pass: resolve forward references and render citations now
        // that every label is known and the citation style/database is final.
        self.resolve_references_in_blocks(&mut blocks);
        let footnote_inlines = std::mem::take(&mut self.footnote_inlines);
        let mut footnote_nums: Vec<usize> = footnote_inlines.keys().copied().collect();
        footnote_nums.sort();
        let mut footnote_inlines = footnote_inlines;
        for num in footnote_nums {
            let mut inlines = footnote_inlines.remove(&num).unwrap();
            self.resolve_references_in_inlines(&mut inlines);
            let text = crate::converter::MarkdownConverter::render_inlines_fragment(&inlines);
            self.state.add_footnote_text(num, text);
        }

        let bibliography = self.render_bibliography();

        Ok(Document {
            metadata: self.metadata.clone(),
            blocks,
            footnotes: self.state.footnote_texts.clone(),
            bibliography,
            warnings: std::mem::take(&mut self.state.warnings),
        })
    }

    /// Walk all blocks, replacing resolvable `Inline::Ref`s with their text
    /// and rendering `Inline::Cite`s against the loaded bibliography.
    fn resolve_references_in_blocks(&mut self, blocks: &mut [Block]) {
        for block in blocks {
            self.resolve_references_in_block(block);
        }
    }

    fn resolve_references_in_block(&mut self, block: &mut Block) {
        match block {
            Block::Section { title, .. } => self.resolve_references_in_inlines(title),
            Block::Paragraph(inlines) => self.resolve_references_in_inlines(inlines),
            Block::BulletList(items) => {
                for item in items {
                    self.resolve_references_in_blocks(item);
                }
            }
            Block::OrderedList { items, .. } => {
                for item in items {
                    self.resolve_references_in_blocks(item);
                }
            }
            Block::DescriptionList(items) => {
                for (term, description) in items {
                    self.resolve_references_in_inlines(term);
                    self.resolve_references_in_blocks(description);
                }
            }
            Block::Quote(blocks) | Block::Composite(blocks) => {
                self.resolve_references_in_blocks(blocks)
            }
            Block::Table {
                caption,
                headers,
                rows,
                ..
            } => {
                if let Some(caption) = caption {
                    self.resolve_references_in_inlines(caption);
                }
                for cell in headers {
                    self.resolve_references_in_blocks(cell);
                }
                for row in rows {
                    for cell in row {
                        self.resolve_references_in_blocks(cell);
                    }
                }
            }
            Block::Figure { caption, .. } => {
                if let Some(caption) = caption {
                    self.resolve_references_in_inlines(caption);
                }
            }
            Block::TheoremLike { content, title, .. } => {
                if let Some(title) = title {
                    self.resolve_references_in_inlines(title);
                }
                self.resolve_references_in_blocks(content)
            }
            Block::CodeBlock { .. }
            | Block::Verbatim(_)
            | Block::DisplayMath(_)
            | Block::RawBlock(_)
            | Block::HorizontalRule
            | Block::Null => {}
        }
    }

    fn resolve_references_in_inlines(&mut self, inlines: &mut [Inline]) {
        for inline in inlines.iter_mut() {
            match inline {
                Inline::Ref { kind, label } => {
                    let rendered = self.render_reference(&kind.clone(), &label.clone());
                    if matches!(rendered, Inline::Ref { .. }) {
                        for key in label.split(',').map(str::trim).filter(|k| !k.is_empty()) {
                            if self.state.get_label(key).is_none() {
                                let warning =
                                    format!("unresolved reference `\\{}{{{}}}`", kind, key);
                                self.state.warn(warning);
                            }
                        }
                    } else {
                        *inline = rendered;
                    }
                }
                Inline::Cite {
                    citations,
                    mode,
                    prenote,
                    postnote,
                    content,
                } => {
                    if content.is_empty() {
                        if self.citation_manager.has_entries() {
                            for key in citations.iter() {
                                if !self.citation_manager.has_entry(key) {
                                    self.state.warn(format!(
                                        "citation key `{}` not found in bibliography",
                                        key
                                    ));
                                }
                            }
                        }
                        let text = self.citation_manager.format_citation(
                            citations,
                            mode.clone(),
                            prenote.as_deref(),
                            postnote.as_deref(),
                        );
                        *content = vec![Inline::Text(text)];
                    }
                }
                Inline::Emph(content)
                | Inline::Strong(content)
                | Inline::Strikeout(content)
                | Inline::Underline(content)
                | Inline::Superscript(content)
                | Inline::Subscript(content)
                | Inline::SmallCaps(content)
                | Inline::Span { content, .. }
                | Inline::Quoted { content, .. }
                | Inline::Link { text: content, .. }
                | Inline::Image { alt: content, .. } => {
                    self.resolve_references_in_inlines(content)
                }
                Inline::Note(blocks) => self.resolve_references_in_blocks(blocks),
                _ => {}
            }
        }
    }

    fn parse_blocks(&mut self) -> Result<Vec<Block>> {
        let mut blocks = Vec::new();

        while !self.is_at_end() {
            self.skip_whitespace_and_comments();

            if self.is_at_end() {
                break;
            }

            if let Some(block) = self.parse_block()? {
                blocks.push(block);
            }
        }

        Ok(blocks)
    }

    fn load_bibliography_files(&mut self) {
        for bib_spec in self.state.bibliography.clone() {
            for bib_name in bib_spec.split(',').map(str::trim).filter(|name| !name.is_empty()) {
                if let Some(path) = self.resolve_bibliography_path(bib_name) {
                    if let Ok(content) = crate::decode::read_file(&path) {
                        let _ = self.citation_manager.parse_bib_file(&content);
                    }
                }
            }
        }
    }

    fn resolve_bibliography_path(&self, bib_name: &str) -> Option<PathBuf> {
        let mut candidates = vec![PathBuf::from(bib_name)];
        if !bib_name.ends_with(".bib") {
            candidates.push(PathBuf::from(format!("{}.bib", bib_name)));
        }

        for candidate in candidates {
            if candidate.is_absolute() && candidate.exists() {
                return Some(candidate);
            }

            if let Some(base_path) = &self.base_path {
                let path = base_path.join(&candidate);
                if path.exists() {
                    return Some(path);
                }
            }

            if candidate.exists() {
                return Some(candidate);
            }
        }

        None
    }

    fn render_bibliography(&self) -> Option<String> {
        let bibliography = self.citation_manager.generate_bibliography();
        let has_entries = bibliography
            .lines()
            .skip(1)
            .any(|line| !line.trim().is_empty());

        if has_entries {
            Some(bibliography)
        } else {
            None
        }
    }

    fn is_block_command(&self, cmd: &str) -> bool {
        matches!(
            cmd,
            "documentclass"
                | "documentstyle"
                | "par"
                | "noindent"
                | "hline"
                | "hrule"
                | "include"
                | "input"
                | "usepackage"
                | "RequirePackage"
                | "newcommand"
                | "renewcommand"
                | "providecommand"
                | "def"
                | "gdef"
                | "edef"
                | "xdef"
                | "DeclareRobustCommand"
                | "newrobustcmd"
                | "renewrobustcmd"
                | "providerobustcmd"
                | "DeclareMathOperator"
                | "NewDocumentCommand"
                | "RenewDocumentCommand"
                | "ProvideDocumentCommand"
                | "DeclareDocumentCommand"
                | "newenvironment"
                | "renewenvironment"
                | "newtheorem"
                | "newtheorem*"
                | "bibliography"
                | "addbibresource"
                | "bibliographystyle"
                | "title"
                | "author"
                | "date"
                | "graphicspath"
                | "pagestyle"
                | "thispagestyle"
                | "maketitle"
                | "tableofcontents"
        )
    }

    fn parse_block(&mut self) -> Result<Option<Block>> {
        if self.depth >= MAX_NESTING_DEPTH {
            return Ok(self.flatten_current_token().map(|inline| Block::Paragraph(vec![inline])));
        }
        self.depth += 1;
        let consumed_before = self.consumed;
        let result = self.parse_block_inner();
        self.depth -= 1;
        // Progress guarantee: every call consumes at least one token, so no
        // loop over parse_block can ever spin forever
        if self.consumed == consumed_before && !self.is_at_end() {
            self.advance();
        }
        // Material before \begin{document} is set-up, never typeset: it is
        // parsed for its side effects (definitions, metadata) only.
        if self.in_preamble && self.depth == 0 {
            return result.map(|_| None);
        }
        result
    }

    /// Past the nesting limit: consume one token and render it literally,
    /// guaranteeing progress without further recursion.
    fn flatten_current_token(&mut self) -> Option<Inline> {
        let token = self.current_token().clone();
        self.advance();
        match token {
            Token::Text(text) => Some(Inline::Text(text)),
            Token::Whitespace(_) | Token::Newline | Token::ParBreak => Some(Inline::Space),
            Token::InlineMath(math) => Some(Inline::InlineMath(math)),
            Token::Verb(code) => Some(Inline::Code(code)),
            Token::LeftBrace | Token::RightBrace | Token::Comment(_) | Token::Eof => None,
            Token::BeginEnvironment(_) | Token::EndEnvironment(_) | Token::Item => None,
            other => {
                let text = self.token_to_string(&other);
                (!text.is_empty()).then_some(Inline::Text(text))
            }
        }
    }

    fn parse_block_inner(&mut self) -> Result<Option<Block>> {
        let token = self.current_token();

        match token {
            Token::BeginEnvironment(env) => self.parse_environment(env.clone()),
            Token::EndEnvironment(env) if env == "document" => {
                // TeX stops reading at \end{document}
                self.tokens.clear();
                self.document_ended = true;
                Ok(None)
            }
            Token::EndEnvironment(env) if self.state.environments.contains_key(env) => {
                // Closing a user-defined environment: splice its end-definition
                let end_def = self.state.environments[env].end_def.clone();
                let env = env.clone();
                self.advance();
                if self.consume_expansion_budget(&format!("end{}", env)) {
                    self.splice_fragment_tokens(&end_def)?;
                }
                Ok(None)
            }
            Token::Command(cmd) => {
                if self.is_block_command(cmd) {
                    self.parse_command_block(cmd.clone())
                } else {
                    self.parse_paragraph()
                }
            }
            Token::Section(level, title, flags) => {
                let level = *level;
                let flags = *flags;
                let title_text = title.clone();
                self.advance();
                if !flags.starred && !flags.part {
                    self.state.increment_section(level as usize);
                }

                // Titles may contain formatting commands and an embedded \label
                let (title_inlines, mut label) = self.parse_inline_fragment(&title_text)?;

                // A \label may also directly follow the heading
                while matches!(
                    self.current_token(),
                    Token::Whitespace(_) | Token::Newline | Token::Comment(_)
                ) {
                    self.advance();
                }
                if let Token::Label(l) = self.current_token() {
                    label = Some(l.clone());
                    self.advance();
                }

                if let Some(section_label) = &label {
                    let plain_title = self.inlines_to_plain_text(&title_inlines);
                    let top = if self.state.has_chapters { 0 } else { 1 };
                    let label_type = if flags.part {
                        LabelType::Other(String::from("Part"))
                    } else if self.state.appendix && level as usize == top {
                        LabelType::Other(String::from("Appendix"))
                    } else if level == 0 {
                        LabelType::Other(String::from("Chapter"))
                    } else {
                        LabelType::Section
                    };
                    let number = self.state.current_section_number.clone();
                    self.state.add_label_with_number(
                        section_label.clone(),
                        label_type,
                        number,
                        Some(plain_title),
                    );
                }

                Ok(Some(Block::Section {
                    level,
                    title: title_inlines,
                    label,
                }))
            }
            Token::ParBreak => {
                self.advance();
                Ok(None)
            }
            Token::DisplayMath(content) => {
                let content = content.clone();
                self.advance();
                let content = self.expand_macros_in_math(&content);
                Ok(Some(Block::DisplayMath(content)))
            }
            Token::VerbatimEnv { name, content } => {
                let name = name.clone();
                let content = content.clone();
                self.advance();
                Ok(self.verbatim_env_to_block(&name, content))
            }
            Token::MathEnv { name, content } => {
                let name = name.clone();
                let content = content.clone();
                self.advance();
                Ok(Some(self.math_env_block(&name, &content)))
            }
            Token::Text(_)
            | Token::InlineMath(_)
            | Token::LeftBrace
            | Token::Ref { .. }
            | Token::Cite { .. }
            | Token::Verb(_)
            | Token::Tilde
            | Token::LeftBracket
            | Token::RightBracket
            | Token::Backslash => self.parse_paragraph(),
            _ => {
                self.advance();
                Ok(None)
            }
        }
    }

    /// Turn a raw-captured verbatim-like environment into a block.
    fn verbatim_env_to_block(&self, name: &str, content: String) -> Option<Block> {
        // Strip at most one leading newline (the one right after \begin{...})
        let body = content.strip_prefix('\n').unwrap_or(&content).to_string();

        match name {
            "comment" => None,
            "lstlisting" => {
                // Optional [key=value,...] options may lead the body
                let (options, code) = split_leading_bracket_group(&body);
                let language = options.as_deref().and_then(extract_listing_language);
                Some(Block::CodeBlock {
                    language,
                    content: code,
                })
            }
            "minted" => {
                // Language is a mandatory {lang} argument after \begin{minted}
                let (options, rest) = split_leading_bracket_group(&body);
                let _ = options;
                let (language, code) = split_leading_brace_group(&rest);
                Some(Block::CodeBlock {
                    language: language.filter(|l| !l.is_empty()),
                    content: code,
                })
            }
            // Graphics code: kept whole (with its \begin/\end) so it can be
            // re-rendered, as a LaTeX code block
            "tikzpicture" | "tikzcd" | "pgfpicture" | "circuitikz" | "pspicture" | "forest"
            | "tikzonimage" | "dot2tex" => Some(Block::CodeBlock {
                language: Some(String::from("latex")),
                content: format!("\\begin{{{}}}{}\\end{{{}}}", name, content, name),
            }),
            "asy" | "asydef" => Some(Block::CodeBlock {
                language: Some(String::from("asymptote")),
                content: body,
            }),
            "gnuplot" => Some(Block::CodeBlock {
                language: Some(String::from("gnuplot")),
                content: body,
            }),
            "mpost" => Some(Block::CodeBlock {
                language: Some(String::from("metapost")),
                content: body,
            }),
            "sagesilent" | "sageblock" | "pycode" => Some(Block::CodeBlock {
                language: Some(String::from(if name.starts_with("sage") { "sage" } else { "python" })),
                content: body,
            }),
            "luacode" | "luacode*" => Some(Block::CodeBlock {
                language: Some(String::from("lua")),
                content: body,
            }),
            "comment*" => None,
            "filecontents" | "filecontents*" => {
                let (_, code) = split_leading_brace_group(&body);
                Some(Block::CodeBlock {
                    language: None,
                    content: code,
                })
            }
            _ => Some(Block::Verbatim(body)),
        }
    }

    fn parse_environment(&mut self, env_name: String) -> Result<Option<Block>> {
        self.advance(); // consume BeginEnvironment

        match env_name.as_str() {
            "document" => {
                // The body starts: content is typeset from here on
                self.in_preamble = false;
                Ok(None)
            }
            "abstract" => self.parse_abstract(),
            "itemize" => self.parse_itemize(),
            "enumerate" => self.parse_enumerate(),
            "description" => self.parse_description(),
            "quote" | "quotation" | "displayquote" | "verse" => {
                self.parse_quote_environment(&env_name)
            }
            "verbatim" => self.parse_verbatim(),
            "lstlisting" | "minted" => self.parse_code_block(&env_name),
            "equation" | "equation*" | "align" | "align*" | "gather" | "gather*" | "multline"
            | "multline*" | "flalign" | "flalign*" | "alignat" | "alignat*" | "eqnarray"
            | "eqnarray*" | "displaymath" | "math" | "split" => {
                self.parse_math_environment(&env_name)
            }
            "table" | "table*" | "sidewaystable" | "sidewaystable*" | "threeparttable"
            | "threeparttablex" | "wraptable" | "margintable" | "SCtable" => {
                self.parse_table_environment(&env_name)
            }
            name if tables::is_tabular_env(name) => self.parse_tabular(&env_name),
            "figure" | "figure*" | "sidewaysfigure" | "wrapfigure" => {
                self.parse_figure_environment(&env_name)
            }
            "center" | "flushleft" | "flushright" | "sloppypar" | "samepage" | "titlepage"
            | "singlespace" | "landscape" | "footnotesize" | "small" | "large" => {
                self.parse_transparent_environment(&env_name, 0, false)
            }
            "minipage" => self.parse_transparent_environment(&env_name, 1, true),
            "multicols" | "spacing" => self.parse_transparent_environment(&env_name, 1, false),
            // Definition containers: their \acro / \newglossaryentry
            // commands are recorded, nothing is typeset here
            "acronym" => self.parse_transparent_environment(&env_name, 0, true),
            "tablenotes" | "TableNotes" | "tnotes" => self.parse_list(&env_name, false),
            "keywords" | "IEEEkeywords" | "keyword" => {
                let content = self.read_environment_inlines(&env_name)?;
                let mut line = vec![Inline::Strong(vec![Inline::Text(String::from("Keywords:"))])];
                if !content.is_empty() {
                    line.push(Inline::Space);
                    line.extend(content);
                }
                Ok(Some(Block::Paragraph(line)))
            }
            // beamer
            "frame" => self.parse_beamer_frame(),
            "columns" => self.parse_transparent_environment(&env_name, 0, true),
            "column" => self.parse_transparent_environment(&env_name, 1, true),
            "block" | "alertblock" | "exampleblock" => self.parse_beamer_block(&env_name),
            "onlyenv" | "visibleenv" | "uncoverenv" | "altenv" | "overprint" | "actionenv" => {
                self.skip_overlay_spec();
                self.parse_transparent_environment(&env_name, 0, true)
            }
            // algorithm floats (algorithm2e's procedure/function too)
            "algorithm" | "algorithm*" | "algorithm2e" | "procedure" | "function" => {
                self.parse_algorithm_float(&env_name)
            }
            "algorithmic" | "algorithmic*" | "algorithmicx" => self.parse_algorithmic(&env_name),
            "thebibliography" => self.parse_thebibliography(),
            "tikzpicture" | "tikzcd" | "pgfpicture" => {
                // TikZ/PGF graphics - preserve as LaTeX code block
                let content = self.read_until_end_environment(&env_name)?;
                Ok(Some(Block::CodeBlock {
                    language: Some("latex".to_string()),
                    content,
                }))
            }
            _ => {
                if self.state.get_theorem_env(&env_name).is_some() {
                    return self.parse_theorem_like(&env_name);
                }
                if self.state.environments.contains_key(&env_name) {
                    return self.expand_user_environment_begin(&env_name);
                }
                // Any declaration works as an environment: \begin{em} = {\em ...}
                if let Some(style) = declaration_style(&env_name) {
                    let blocks = self.parse_environment_blocks(&env_name)?;
                    let blocks = blocks
                        .into_iter()
                        .map(|block| style_block(&style, block))
                        .collect::<Vec<_>>();
                    return Ok(match blocks.len() {
                        0 => None,
                        1 => blocks.into_iter().next(),
                        _ => Some(Block::Composite(blocks)),
                    });
                }
                // \begin{foo} runs \foo when foo is a macro or a known
                // command without output (\begin{sloppypar}, \begin{small})
                if self.state.macros.contains_key(&env_name) {
                    self.push_front(vec![Token::Command(env_name.clone())]);
                    let blocks = self.parse_environment_blocks(&env_name)?;
                    return Ok(match blocks.len() {
                        0 => None,
                        1 => blocks.into_iter().next(),
                        _ => Some(Block::Composite(blocks)),
                    });
                }
                if noop_command_signature(&env_name).is_some_and(|(args, _)| args == 0)
                    || knowledge::setup_command(&env_name).is_some_and(|s| s.braced == 0)
                {
                    return self.parse_transparent_environment(&env_name, 0, false);
                }
                // Theorem-like names whose \newtheorem lives in a style file
                // we could not read: treat them as theorems anyway
                if let Some(display) = knowledge::implicit_theorem_name(&env_name) {
                    self.state.define_theorem_env(
                        env_name.clone(),
                        display.to_string(),
                        !env_name.ends_with('*'),
                        Some(env_name.trim_end_matches('*').to_string()),
                        None,
                    );
                    return self.parse_theorem_like(&env_name);
                }
                // Preserve unsupported environments as raw LaTeX.
                self.state.warn(format!(
                    "unknown environment `{}` preserved as raw LaTeX",
                    env_name
                ));
                let content = self.read_until_end_environment(&env_name)?;
                Ok(Some(Block::RawBlock(format!(
                    "\\begin{{{}}}{}\\end{{{}}}",
                    env_name, content, env_name
                ))))
            }
        }
    }

    /// Expand the begin-definition of a \newenvironment environment by
    /// substituting arguments and splicing the lexed body into the token
    /// stream. The matching \end{name} splices the end-definition the same
    /// way (see parse_block), so nested structures compose naturally.
    fn expand_user_environment_begin(&mut self, env_name: &str) -> Result<Option<Block>> {
        let Some(def) = self.state.environments.get(env_name).cloned() else {
            return Ok(None);
        };

        let mut args = Vec::new();
        let required = if let Some(default) = &def.optional_param {
            self.skip_whitespace_and_comments();
            if matches!(self.current_token(), Token::LeftBracket) {
                args.push(self.parse_bracketed_text()?);
            } else {
                args.push(default.clone());
            }
            def.num_params.saturating_sub(1)
        } else {
            def.num_params
        };
        for _ in 0..required {
            self.skip_whitespace_and_comments();
            args.push(self.parse_braced_text()?);
        }

        let body = crate::state::substitute_params(&def.begin_def, &args);
        if !self.consume_expansion_bytes(env_name, body.len()) || !self.consume_expansion_budget(env_name) {
            return Ok(None);
        }
        self.splice_fragment_tokens(&body)?;
        Ok(None)
    }

    /// Lex a LaTeX fragment and splice its tokens at the current position.
    fn splice_fragment_tokens(&mut self, fragment: &str) -> Result<()> {
        if fragment.trim().is_empty() {
            return Ok(());
        }
        self.push_front(tex::lex_fragment(fragment));
        Ok(())
    }

    /// Parse an environment whose contents should simply flow into the
    /// document (center, minipage, ...), consuming any leading arguments.
    fn parse_transparent_environment(
        &mut self,
        env_name: &str,
        braced_args: usize,
        optional_arg: bool,
    ) -> Result<Option<Block>> {
        if optional_arg {
            self.skip_whitespace_and_comments();
            if matches!(self.current_token(), Token::LeftBracket) {
                let _ = self.parse_bracketed_text()?;
            }
        }
        for _ in 0..braced_args {
            self.skip_whitespace_and_comments();
            let _ = self.parse_braced_text()?;
        }

        let mut blocks = Vec::new();
        while !self.is_at_end() {
            self.skip_whitespace_and_comments();

            if let Token::EndEnvironment(env) = self.current_token() {
                if env == env_name {
                    self.advance();
                    break;
                }
            }

            match self.parse_block()? {
                Some(block) => blocks.push(block),
                None => continue,
            }
        }

        match blocks.len() {
            0 => Ok(None),
            1 => Ok(Some(blocks.into_iter().next().unwrap())),
            _ => Ok(Some(Block::Composite(blocks))),
        }
    }

    /// Parse `\begin{thebibliography}{widest} \bibitem{key} ... \end{thebibliography}`
    /// into a References section with numbered entries.
    fn parse_thebibliography(&mut self) -> Result<Option<Block>> {
        self.skip_whitespace_and_comments();
        let _ = self.parse_braced_text()?; // {widest-label} argument

        let mut entries: Vec<(String, Vec<Inline>)> = Vec::new();
        let mut current: Option<(String, Vec<Inline>)> = None;

        while !self.is_at_end() {
            match self.current_token() {
                Token::EndEnvironment(env) if env == "thebibliography" => {
                    self.advance();
                    break;
                }
                Token::Command(cmd) if cmd == "bibitem" => {
                    if let Some(entry) = current.take() {
                        entries.push(entry);
                    }
                    self.advance();
                    if matches!(self.current_token(), Token::LeftBracket) {
                        let _ = self.parse_bracketed_text()?;
                    }
                    let key = self.parse_braced_text()?;
                    current = Some((key, Vec::new()));
                }
                Token::ParBreak => {
                    self.advance();
                    if let Some((_, inlines)) = current.as_mut() {
                        if !inlines.is_empty() {
                            inlines.push(Inline::Space);
                        }
                    }
                }
                _ => {
                    let inline = self.parse_inline()?;
                    if let (Some(inline), Some((_, inlines))) = (inline, current.as_mut()) {
                        inlines.push(inline);
                    }
                }
            }
        }
        if let Some(entry) = current.take() {
            entries.push(entry);
        }

        if entries.is_empty() {
            return Ok(None);
        }

        let mut blocks = vec![Block::Section {
            level: 1,
            title: vec![Inline::Text(String::from("References"))],
            label: None,
        }];

        for (key, inlines) in entries {
            let marker = self.citation_manager.format_citation(
                &[key],
                CitationMode::Normal,
                None,
                None,
            );
            let mut paragraph = vec![Inline::Text(format!("{} ", marker))];
            paragraph.extend(inlines);
            blocks.push(Block::Paragraph(paragraph));
        }

        Ok(Some(Block::Composite(blocks)))
    }

    fn parse_itemize(&mut self) -> Result<Option<Block>> {
        self.parse_list("itemize", false)
    }

    fn parse_enumerate(&mut self) -> Result<Option<Block>> {
        self.parse_list("enumerate", true)
    }

    /// Parse itemize/enumerate-like environments. Custom item labels
    /// (`\item[(iii)]`) are kept as leading text of the item, since Markdown
    /// lists cannot express arbitrary markers.
    fn parse_list(&mut self, env_name: &str, ordered: bool) -> Result<Option<Block>> {
        self.list_depth += 1;
        let result = self.parse_list_inner(env_name, ordered);
        self.list_depth -= 1;
        // LaTeX rejects lists nested more than 4 deep; past a generous limit,
        // flatten instead of producing ever-deeper Markdown indentation.
        if self.list_depth >= MAX_LIST_NESTING {
            if let Ok(Some(Block::BulletList(items) | Block::OrderedList { items, .. })) = result {
                return Ok(Some(Block::Composite(items.into_iter().flatten().collect())));
            }
        }
        result
    }

    fn parse_list_inner(&mut self, env_name: &str, ordered: bool) -> Result<Option<Block>> {
        let mut items: Vec<Vec<Block>> = Vec::new();
        let mut custom_labels = 0usize;
        let mut start = 1usize;
        self.pending_enum_start = None;

        // Optional list options: \begin{enumerate}[label=(\alph*), start=3]
        self.skip_whitespace_and_comments();
        if matches!(self.current_token(), Token::LeftBracket) {
            let options = self.parse_bracketed_text()?;
            for option in options.split(',') {
                let mut parts = option.splitn(2, '=');
                if parts.next().map(str::trim) == Some("start") {
                    if let Some(n) = parts.next().and_then(|v| v.trim().parse::<usize>().ok()) {
                        start = n;
                    }
                }
            }
        }

        while !self.is_at_end() {
            self.skip_whitespace_and_comments();

            match self.current_token() {
                Token::EndEnvironment(env) if env == env_name => {
                    self.advance();
                    break;
                }
                // A mismatched \end closes the list too (unbalanced input)
                Token::EndEnvironment(env)
                    if !self.state.environments.contains_key(env) =>
                {
                    break;
                }
                Token::Item => {
                    self.advance();
                    self.skip_overlay_spec();
                    let label = self.parse_item_label()?;
                    if items.is_empty() {
                        if let Some(n) = self.pending_enum_start.take() {
                            start = n;
                        }
                    }
                    let mut item_blocks = self.parse_item_content(env_name)?;
                    if let Some(label) = label.filter(|l| !l.is_empty()) {
                        custom_labels += 1;
                        prepend_item_label(&mut item_blocks, label);
                    }
                    items.push(item_blocks);
                }
                Token::Command(_) => {
                    // Content before the first \item (\setcounter, \small, ...)
                    let _ = self.parse_inline()?;
                }
                Token::BeginEnvironment(_) | Token::MathEnv { .. } | Token::VerbatimEnv { .. }
                | Token::DisplayMath(_) => {
                    // Blocks before the first \item: attach to a leading item
                    if let Some(block) = self.parse_block()? {
                        match items.last_mut() {
                            Some(item) => item.push(block),
                            None => items.push(vec![block]),
                        }
                    }
                }
                _ => self.advance(),
            }
        }

        if ordered && (custom_labels == 0 || custom_labels < items.len()) {
            Ok(Some(Block::OrderedList { start, items }))
        } else {
            Ok(Some(Block::BulletList(items)))
        }
    }

    /// Parse an optional `[label]` right after `\item` as inlines.
    fn parse_item_label(&mut self) -> Result<Option<Vec<Inline>>> {
        let mut lookahead = 0;
        while matches!(
            self.peek_at(lookahead),
            Some(Token::Whitespace(_)) | Some(Token::Newline)
        ) {
            lookahead += 1;
        }
        if !matches!(self.peek_at(lookahead), Some(Token::LeftBracket)) {
            return Ok(None);
        }
        self.skip_n(lookahead);
        let tokens = self.collect_bracketed_tokens();
        let (inlines, _) = self.parse_inline_fragment_tokens(tokens)?;
        Ok(Some(trim_inline_whitespace(inlines)))
    }

    /// Collect the raw tokens of a `[...]` group (brace-aware), consuming it.
    fn collect_bracketed_tokens(&mut self) -> Vec<Token> {
        if !matches!(self.current_token(), Token::LeftBracket) {
            return Vec::new();
        }
        self.advance();
        let mut depth = 0usize;
        let mut tokens = Vec::new();
        while !self.is_at_end() {
            match self.current_token() {
                Token::RightBracket if depth == 0 => {
                    self.advance();
                    break;
                }
                Token::ParBreak => break,
                Token::LeftBrace => depth += 1,
                Token::RightBrace => depth = depth.saturating_sub(1),
                _ => {}
            }
            tokens.push(self.current_token().clone());
            self.advance();
        }
        tokens
    }

    fn parse_description(&mut self) -> Result<Option<Block>> {
        let mut items = Vec::new();

        while !self.is_at_end() {
            self.skip_whitespace_and_comments();

            if let Token::EndEnvironment(env) = self.current_token() {
                if env == "description" {
                    self.advance();
                    break;
                }
            }

            if matches!(self.current_token(), Token::Item) {
                self.advance();
                
                // Optional term in brackets (brace-aware: \item[{a [b] c}])
                let term = self.parse_item_label()?.unwrap_or_default();

                let item_blocks = self.parse_item_content("description")?;
                items.push((term, item_blocks));
            } else {
                self.advance();
            }
        }

        Ok(Some(Block::DescriptionList(items)))
    }

    fn parse_item_content(&mut self, env_name: &str) -> Result<Vec<Block>> {
        let mut blocks = Vec::new();

        // Parse inline content until we hit a block boundary
        let inlines = self.parse_inlines_until_item_or_end(env_name)?;
        
        if !inlines.is_empty() {
            blocks.push(Block::Paragraph(inlines));
        }

        // Then parse any additional blocks
        while !self.is_at_end() {
            self.skip_whitespace_and_comments();

            // Stop at next item or end of environment
            if matches!(self.current_token(), Token::Item) {
                break;
            }

            if let Token::EndEnvironment(env) = self.current_token() {
                if env == env_name {
                    break;
                }
            }

            match self.parse_block()? {
                Some(block) => blocks.push(block),
                None => break,
            }
        }

        if blocks.is_empty() {
            blocks.push(Block::Paragraph(vec![]));
        }

        Ok(blocks)
    }
    
    fn parse_inlines_until_item_or_end(&mut self, env_name: &str) -> Result<Vec<Inline>> {
        let mut inlines = Vec::new();

        while !self.is_at_end() {
            let token = self.current_token();

            match token {
                Token::Item => break,
                Token::EndEnvironment(env) if env == env_name => break,
                Token::ParBreak
                | Token::BeginEnvironment(_)
                | Token::DisplayMath(_)
                | Token::MathEnv { .. }
                | Token::Section(..)
                | Token::VerbatimEnv { .. } => break,
                _ => {
                    if let Some(inline) = self.parse_inline()? {
                        inlines.push(inline);
                    }
                }
            }
        }

        Ok(inlines)
    }

    fn parse_abstract(&mut self) -> Result<Option<Block>> {
        let mut blocks = Vec::new();

        while !self.is_at_end() {
            self.skip_whitespace_and_comments();

            if let Token::EndEnvironment(env) = self.current_token() {
                if env == "abstract" {
                    self.advance();
                    break;
                }
            }

            match self.parse_block()? {
                Some(block) => blocks.push(block),
                None => continue,
            }
        }

        // Wrap abstract in a quote block with a header
        let mut abstract_blocks = vec![
            Block::Paragraph(vec![Inline::Strong(vec![Inline::Text("Abstract".to_string())])])
        ];
        abstract_blocks.extend(blocks);
        
        Ok(Some(Block::Quote(abstract_blocks)))
    }

    fn parse_quote_environment(&mut self, env_name: &str) -> Result<Option<Block>> {
        let mut blocks = Vec::new();

        while !self.is_at_end() {
            self.skip_whitespace_and_comments();

            if let Token::EndEnvironment(env) = self.current_token() {
                if env == env_name {
                    self.advance();
                    break;
                }
            }

            match self.parse_block()? {
                Some(block) => blocks.push(block),
                None => continue,
            }
        }

        Ok(Some(Block::Quote(blocks)))
    }

    fn parse_verbatim(&mut self) -> Result<Option<Block>> {
        let content = self.read_until_end_environment("verbatim")?;
        Ok(Some(Block::Verbatim(content)))
    }

    fn parse_code_block(&mut self, env_name: &str) -> Result<Option<Block>> {
        // Try to extract language from optional argument
        let language = if matches!(self.current_token(), Token::LeftBracket) {
            self.advance();
            let lang = self.read_until_token(Token::RightBracket);
            if matches!(self.current_token(), Token::RightBracket) {
                self.advance();
            }
            Some(lang)
        } else {
            None
        };

        let content = self.read_until_end_environment(env_name)?;

        Ok(Some(Block::CodeBlock { language, content }))
    }

    fn parse_math_environment(&mut self, env_name: &str) -> Result<Option<Block>> {
        // Token-based fallback (the lexer normally captures math environments
        // raw as Token::MathEnv; this path handles bodies assembled from
        // macro/environment expansions).
        let mut content = String::new();
        while !self.is_at_end() {
            if let Token::EndEnvironment(env) = self.current_token() {
                if env == env_name {
                    self.advance();
                    break;
                }
            }
            content.push_str(&self.token_to_string(self.current_token()));
            self.advance();
        }
        Ok(Some(self.math_env_block(env_name, &content)))
    }

    /// Convert the raw body of a display-math environment into a block:
    /// assigns equation numbers (per row for align-like environments),
    /// registers \label targets, strips constructs Markdown math renderers
    /// reject, and wraps multi-row environments in aligned/gathered.
    fn math_env_block(&mut self, env_name: &str, raw: &str) -> Block {
        let base = env_name.trim_end_matches('*');
        let starred = env_name.ends_with('*');
        let mut body = raw.trim().to_string();

        // alignat{n} / flalign column count argument
        if base == "alignat" {
            if let (Some(_), rest) = split_leading_brace_group(&body) {
                body = rest.trim().to_string();
            }
        }

        let numbered = !starred && !matches!(base, "displaymath" | "math");
        let per_row = matches!(base, "align" | "gather" | "eqnarray" | "flalign" | "alignat");

        let mut rows = split_math_rows(&body);
        // A trailing \\ leaves an empty final row that LaTeX does not number
        if rows.len() > 1 && rows.last().is_some_and(|(row, _)| row.trim().is_empty()) {
            rows.pop();
        }

        let mut tags: Vec<String> = Vec::new();
        let mut whole_labels: Vec<String> = Vec::new();
        let mut whole_nonumber = false;
        let mut whole_tag: Option<String> = None;
        let mut out_rows: Vec<(String, String)> = Vec::new();

        for (row, sep) in rows {
            let mut row_info = strip_math_row_markup(&row);
            if base == "eqnarray" {
                row_info.text = eqnarray_row_to_aligned(&row_info.text);
            }
            if per_row {
                let has_content = !row_info.text.trim().is_empty();
                if let Some(tag) = row_info.tag.take() {
                    for label in &row_info.labels {
                        self.state.add_label_with_number(label.clone(), LabelType::Equation, tag.clone(), None);
                    }
                    tags.push(tag);
                } else if numbered && !row_info.nonumber && has_content {
                    let number = self.state.next_counter("equation");
                    if !row_info.labels.is_empty() {
                        for label in &row_info.labels {
                            self.state.add_label_with_number(label.clone(), LabelType::Equation, number.clone(), None);
                        }
                        tags.push(number);
                    }
                } else {
                    for label in &row_info.labels {
                        self.state.add_label_with_number(label.clone(), LabelType::Equation, String::new(), None);
                    }
                }
            } else {
                whole_labels.extend(row_info.labels.drain(..));
                whole_nonumber |= row_info.nonumber;
                if row_info.tag.is_some() {
                    whole_tag = row_info.tag.take();
                }
            }
            out_rows.push((row_info.text, sep));
        }

        if !per_row {
            if let Some(tag) = whole_tag {
                for label in &whole_labels {
                    self.state.add_label_with_number(label.clone(), LabelType::Equation, tag.clone(), None);
                }
                tags.push(tag);
            } else if numbered && !whole_nonumber {
                let number = self.state.next_counter("equation");
                for label in &whole_labels {
                    self.state.add_label_with_number(label.clone(), LabelType::Equation, number.clone(), None);
                }
                if !whole_labels.is_empty() {
                    tags.push(number);
                }
            } else {
                for label in &whole_labels {
                    self.state.add_label_with_number(label.clone(), LabelType::Equation, String::new(), None);
                }
            }
        }

        let multi_row = out_rows.len() > 1;
        let mut joined = String::new();
        for (i, (row, sep)) in out_rows.iter().enumerate() {
            let row = row.trim();
            joined.push_str(row);
            if i + 1 < out_rows.len() {
                joined.push(' ');
                joined.push_str(sep);
                joined.push('\n');
            }
        }
        let mut content = self.expand_macros_in_math(joined.trim());

        let wrapper = match base {
            "align" | "alignat" | "flalign" | "eqnarray" => Some("aligned"),
            "multline" => Some("gathered"),
            "gather" => Some("gathered"),
            _ => None,
        };
        if let Some(wrapper) = wrapper {
            if multi_row || content.contains('&') {
                content = format!("\\begin{{{}}}\n{}\n\\end{{{}}}", wrapper, content, wrapper);
            }
        }
        if !tags.is_empty() {
            content = format!("{}\n\\tag{{{}}}", content, tags.join(", "));
        }
        Block::DisplayMath(content)
    }

    fn parse_table_environment(&mut self, env_name: &str) -> Result<Option<Block>> {
        self.parse_table_float(env_name)
    }

    fn parse_tabular(&mut self, env_name: &str) -> Result<Option<Block>> {
        self.parse_standalone_tabular(env_name)
    }

    /// Consume the arguments of table rule commands like \cline{2-3} or
    /// \cmidrule(lr){2-3}, which were already identified by the caller.
    fn consume_table_rule_args(&mut self, cmd: &str) -> Result<()> {
        if matches!(cmd, "cline" | "cmidrule" | "cmidrules") {
            // \cmidrule may carry (lr) trimming, lexed as plain text
            if let Token::Text(text) = self.current_token() {
                if text.starts_with('(') {
                    self.advance();
                }
            }
            if matches!(self.current_token(), Token::LeftBracket) {
                let _ = self.parse_bracketed_text()?;
            }
            if matches!(self.current_token(), Token::LeftBrace) {
                let _ = self.parse_braced_text()?;
            }
        }
        if matches!(cmd, "rowcolor" | "arrayrulecolor")
            && matches!(self.current_token(), Token::LeftBrace) {
                let _ = self.parse_braced_text()?;
            }
        if cmd == "addlinespace" && matches!(self.current_token(), Token::LeftBracket) {
            let _ = self.parse_bracketed_text()?;
        }
        Ok(())
    }

    fn parse_column_spec(&self, spec: &str) -> Vec<Alignment> {
        let mut alignments = Vec::new();
        let chars: Vec<char> = spec.chars().collect();
        let mut i = 0;

        fn skip_braced(chars: &[char], mut i: usize) -> (String, usize) {
            let mut content = String::new();
            if i < chars.len() && chars[i] == '{' {
                let mut depth = 1;
                i += 1;
                while i < chars.len() {
                    match chars[i] {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                i += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    if depth > 0 {
                        content.push(chars[i]);
                    }
                    i += 1;
                }
            }
            (content, i)
        }

        while i < chars.len() {
            let ch = chars[i];
            i += 1;
            match ch {
                'l' => alignments.push(Alignment::AlignLeft),
                'c' => alignments.push(Alignment::AlignCenter),
                'r' => alignments.push(Alignment::AlignRight),
                'p' | 'm' | 'b' | 'X' => {
                    alignments.push(Alignment::AlignLeft);
                    let (_, next) = skip_braced(&chars, i);
                    i = next;
                }
                '@' | '>' | '<' | '!' => {
                    let (_, next) = skip_braced(&chars, i);
                    i = next;
                }
                '*' => {
                    // *{n}{spec} column repetition
                    let (count, next) = skip_braced(&chars, i);
                    let (inner, next) = skip_braced(&chars, next);
                    i = next;
                    let count = count.trim().parse::<usize>().unwrap_or(0);
                    let inner_alignments = self.parse_column_spec(&inner);
                    for _ in 0..count {
                        if alignments.len() + inner_alignments.len() > MAX_TABLE_COLUMNS {
                            break;
                        }
                        alignments.extend(inner_alignments.iter().cloned());
                    }
                }
                _ => {}
            }
        }

        alignments.truncate(MAX_TABLE_COLUMNS);
        alignments
    }

    fn parse_figure_environment(&mut self, env_name: &str) -> Result<Option<Block>> {
        self.parse_figure_float(env_name)
    }

    fn parse_theorem_like(&mut self, env_type: &str) -> Result<Option<Block>> {
        let theorem_def = self
            .state
            .get_theorem_env(env_type)
            .cloned()
            .unwrap_or_else(|| crate::state::TheoremEnvDef {
                display_name: env_type.to_string(),
                numbered: false,
                counter_key: env_type.to_string(),
                within: None,
            });
        let number = self.state.next_theorem_number(env_type);
        // Optional [title], parsed as inline LaTeX (may hold math, \cite, ...)
        let mut label = None;
        let mut lookahead = 0;
        while matches!(self.peek_at(lookahead), Some(Token::Whitespace(_))) {
            lookahead += 1;
        }
        let title = if matches!(self.peek_at(lookahead), Some(Token::LeftBracket)) {
            self.skip_n(lookahead);
            let tokens = self.collect_bracketed_tokens();
            let (inlines, title_label) = self.parse_inline_fragment_tokens(tokens)?;
            if title_label.is_some() {
                label = title_label;
            }
            Some(trim_inline_whitespace(inlines)).filter(|t| !t.is_empty())
        } else {
            None
        };

        let mut content = Vec::new();

        while !self.is_at_end() {
            self.skip_whitespace_and_comments();

            if let Token::EndEnvironment(env) = self.current_token() {
                if env == env_type {
                    self.advance();
                    break;
                }
            }

            if let Token::Label(l) = self.current_token() {
                label = Some(l.clone());
                self.advance();
                continue;
            }

            match self.parse_block()? {
                Some(block) => content.push(block),
                None => continue,
            }
        }

        if let (Some(theorem_label), Some(theorem_number)) = (&label, &number) {
            self.state.add_label_with_number(
                theorem_label.clone(),
                LabelType::Other(theorem_def.display_name.clone()),
                theorem_number.clone(),
                title.as_ref().map(|t| self.inlines_to_plain_text(t)),
            );
        }

        Ok(Some(Block::TheoremLike {
            env_type: env_type.to_string(),
            display_name: theorem_def.display_name,
            number,
            label,
            title,
            content,
        }))
    }

    fn parse_paragraph(&mut self) -> Result<Option<Block>> {
        let inlines = self.parse_inlines_until_block_end()?;

        if inlines.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Block::Paragraph(inlines)))
        }
    }

    fn parse_inlines_until_block_end(&mut self) -> Result<Vec<Inline>> {
        let mut inlines = Vec::new();

        while !self.is_at_end() {
            let token = self.current_token();

            match token {
                Token::ParBreak
                | Token::BeginEnvironment(_)
                | Token::Section(..)
                | Token::DisplayMath(_)
                | Token::MathEnv { .. }
                | Token::VerbatimEnv { .. } => {
                    break;
                }
                Token::EndEnvironment(_) => {
                    break;
                }
                Token::Command(cmd) if cmd == "par" => {
                    self.advance();
                    break;
                }
                // Block-producing commands end the paragraph
                Token::Command(cmd)
                    if matches!(cmd.as_str(), "maketitle" | "hrule")
                        && !self.macro_processor.is_defined(&self.state, cmd) =>
                {
                    break;
                }
                _ => {
                    // A font declaration styles the rest of the paragraph
                    if let Some(style) = self.current_declaration() {
                        self.advance();
                        let rest = trim_leading_space(self.parse_inlines_until_block_end()?);
                        inlines.extend(apply_decl_style(&style, rest));
                        break;
                    }
                    if let Some(inline) = self.parse_inline()? {
                        inlines.push(inline);
                    }
                }
            }
        }

        Ok(inlines)
    }

    fn parse_inlines_until(&mut self, end_token: Token) -> Result<Vec<Inline>> {
        let mut inlines = Vec::new();

        while !self.is_at_end() && self.current_token() != &end_token {
            // A font declaration mid-group (`a \bf b`) styles the rest of it
            if let Some(style) = self.current_declaration() {
                self.advance();
                let rest = self.parse_inlines_until(end_token)?;
                let rest = trim_leading_space(rest);
                inlines.extend(apply_decl_style(&style, rest));
                break;
            }
            if let Some(inline) = self.parse_inline()? {
                inlines.push(inline);
            }
        }

        Ok(inlines)
    }

    /// The font declaration at the cursor, if any (and not redefined).
    fn current_declaration(&self) -> Option<DeclStyle> {
        match self.current_token() {
            Token::Command(cmd) if !self.state.macros.contains_key(cmd) => declaration_style(cmd),
            _ => None,
        }
    }

    fn parse_inline(&mut self) -> Result<Option<Inline>> {
        if self.depth >= MAX_NESTING_DEPTH {
            return Ok(self.flatten_current_token());
        }
        self.depth += 1;
        let consumed_before = self.consumed;
        let result = self.parse_inline_inner();
        self.depth -= 1;
        if self.consumed == consumed_before && !self.is_at_end() {
            self.advance();
        }
        result
    }

    fn parse_inline_inner(&mut self) -> Result<Option<Inline>> {
        let token = self.current_token();

        match token {
            Token::Text(text) => {
                let text = text.clone();
                self.advance();
                Ok(Some(Inline::Text(text)))
            }
            Token::Whitespace(_) | Token::Newline => {
                self.advance();
                Ok(Some(Inline::Space))
            }
            Token::InlineMath(content) => {
                let content = content.clone();
                self.advance();
                let expanded = self.expand_math_with_engine(&content, false);
                Ok(Some(Inline::InlineMath(math::clean_math(&expanded))))
            }
            Token::Command(cmd) => self.parse_inline_command(cmd.clone()),
            Token::LeftBrace => self.parse_braced_group(),
            Token::Verb(code) => {
                let code = code.clone();
                self.advance();
                Ok(Some(Inline::Code(code)))
            }
            // Brackets outside an optional-argument position are literal text
            Token::LeftBracket => {
                self.advance();
                Ok(Some(Inline::Text(String::from("["))))
            }
            Token::RightBracket => {
                self.advance();
                Ok(Some(Inline::Text(String::from("]"))))
            }
            Token::Tilde => {
                self.advance();
                // Non-breaking space
                Ok(Some(Inline::Text(String::from("\u{00A0}"))))
            }
            Token::Backslash => {
                self.advance();
                // Optional vertical spacing: \\[4pt]
                if matches!(self.current_token(), Token::LeftBracket) {
                    let _ = self.parse_bracketed_text()?;
                }
                Ok(Some(Inline::LineBreak))
            }
            Token::Ref { kind, label } => {
                let kind = kind.clone();
                let label = label.clone();
                self.advance();
                Ok(Some(self.render_reference(&kind, &label)))
            }
            Token::Cite { kind, keys } => {
                let kind = kind.clone();
                let citation = keys.clone();
                self.advance();
                let keys = citation
                    .split(',')
                    .map(str::trim)
                    .filter(|key| !key.is_empty())
                    .map(ToString::to_string)
                    .collect::<Vec<_>>();
                Ok(Some(Inline::Cite {
                    citations: keys,
                    mode: citation_mode_for(&kind),
                    prenote: None,
                    postnote: None,
                    content: Vec::new(),
                }))
            }
            _ => {
                self.advance();
                Ok(None)
            }
        }
    }

    fn parse_inline_command(&mut self, cmd: String) -> Result<Option<Inline>> {
        self.advance();

        // Built-in meaning captured by \let: skip user macros and aliases
        if let Some(builtin) = cmd.strip_prefix(tex::BUILTIN_MARK) {
            return self.parse_builtin_command(builtin.to_string());
        }

        // TeX-level constructs (conditionals, \let aliases, \csname, ...);
        // user macros take precedence over everything but aliases.
        if !self.macro_processor.is_defined(&self.state, &cmd) || self.state.aliases.contains_key(&cmd) {
            if let Some(result) = self.parse_tex_command(&cmd)? {
                return Ok(result);
            }
        }

        // Definitions and preamble commands appearing mid-paragraph
        if is_inline_safe_block_command(&cmd) {
            self.push_front(vec![Token::Command(cmd.clone())]);
            let _ = self.parse_command_block(cmd)?;
            return Ok(None);
        }

        // \input inside a paragraph (or a table cell, list item, ...): splice
        // the file's tokens in place, as TeX does
        if cmd == "input" || cmd == "include" || cmd == "subfile" || cmd == "import" {
            self.splice_input_file(&cmd)?;
            return Ok(None);
        }

        // Check for citation commands
        if self.is_citation_command(&cmd) {
            return self.parse_citation_command(&cmd);
        }

        // Check for reference commands
        if self.is_reference_command(&cmd) {
            return self.parse_reference_command(&cmd);
        }
        if cmd == "crefrange" || cmd == "Crefrange" {
            self.consume_text_star();
            let first = self.parse_braced_text()?;
            self.skip_whitespace_only();
            let last = self.parse_braced_text()?;
            let (a, b) = (first.trim(), last.trim());
            match (self.state.get_label(a), self.state.get_label(b)) {
                (Some(start), Some(end)) => {
                    let (name, parens) =
                        ref_type_name(start, cmd == "Crefrange", true, RefStyle::Cleveref);
                    let wrap = |n: &str| if parens { format!("({})", n) } else { n.to_string() };
                    return Ok(Some(Inline::Text(format!(
                        "{} {} to {}",
                        name,
                        wrap(&start.number),
                        wrap(&end.number)
                    ))));
                }
                _ => {
                    // Forward references: resolve as a list in the second pass
                    return Ok(Some(Inline::Ref {
                        kind: String::from(if cmd == "Crefrange" { "Cref" } else { "cref" }),
                        label: format!("{},{}", a, b),
                    }));
                }
            }
        }

        // Check for footnote commands
        if cmd == "footnote" || cmd == "thanks" {
            if matches!(self.current_token(), Token::LeftBracket) {
                let _ = self.parse_bracketed_text()?;
            }
            let tokens = self.collect_braced_tokens();
            let (inlines, _) = self.parse_inline_fragment_tokens(tokens)?;
            let num = self.state.add_footnote_mark(String::new());
            self.footnote_inlines.insert(num, inlines);
            return Ok(Some(Inline::Text(format!("[^{}]", num))));
        }
        if cmd == "footnotemark" {
            let explicit = if matches!(self.current_token(), Token::LeftBracket) {
                self.parse_bracketed_text()?.trim().parse::<usize>().ok()
            } else {
                None
            };
            let num = match explicit {
                Some(n) => n,
                None => self.state.add_footnote_mark(String::new()),
            };
            return Ok(Some(Inline::Text(format!("[^{}]", num))));
        }
        if cmd == "footnotetext" {
            let explicit = if matches!(self.current_token(), Token::LeftBracket) {
                self.parse_bracketed_text()?.trim().parse::<usize>().ok()
            } else {
                None
            };
            let tokens = self.collect_braced_tokens();
            let (inlines, _) = self.parse_inline_fragment_tokens(tokens)?;
            let num = explicit.unwrap_or(self.state.footnote_counter);
            self.footnote_inlines.insert(num, inlines);
            return Ok(None);
        }

        // Check if it's a user-defined macro
        if self.macro_processor.is_defined(&self.state, &cmd) {
            return self.parse_macro_expansion(&cmd);
        }

        self.parse_builtin_command(cmd)
    }

    /// Commands with built-in meaning (everything but user macros).
    fn parse_builtin_command(&mut self, cmd: String) -> Result<Option<Inline>> {

        // Counter manipulation affects numbering of later sections, floats,
        // equations and list items
        match cmd.as_str() {
            "appendix" => {
                self.state.start_appendix();
                return Ok(None);
            }
            "setcounter" | "addtocounter" => {
                self.skip_whitespace_and_comments();
                let name = self.parse_braced_text()?.trim().to_string();
                self.skip_whitespace_and_comments();
                let value_text = self.parse_braced_text()?;
                let value = value_text.trim().parse::<i64>().unwrap_or(0);
                let current = self.counter_value(&name) as i64;
                let new_value = if cmd == "setcounter" { value } else { current + value };
                let new_value = new_value.clamp(0, 1_000_000) as usize;
                if name.starts_with("enum") {
                    self.pending_enum_start = Some(new_value + 1);
                } else {
                    self.state.set_counter(&name, new_value);
                }
                return Ok(None);
            }
            "stepcounter" | "refstepcounter" => {
                self.skip_whitespace_and_comments();
                let name = self.parse_braced_text()?.trim().to_string();
                match name.as_str() {
                    "equation" | "figure" | "table" => {
                        let _ = self.state.next_counter(&name);
                    }
                    _ => {
                        let value = self.counter_value(&name);
                        self.state.set_counter(&name, value + 1);
                    }
                }
                return Ok(None);
            }
            "numberwithin" => {
                self.skip_whitespace_and_comments();
                if matches!(self.current_token(), Token::LeftBracket) {
                    let _ = self.parse_bracketed_text()?;
                }
                let counter = self.parse_braced_text()?.trim().to_string();
                self.skip_whitespace_and_comments();
                let parent = self.parse_braced_text()?.trim().to_string();
                if !counter.is_empty() && crate::state::level_of_sectioning(&parent).is_some() {
                    self.state.numberwithin.insert(counter, parent);
                }
                return Ok(None);
            }
            _ => {}
        }

        // Accents: \'e, \"o, \c{c}, ...
        if let Some(accent) = get_accent_type(&cmd) {
            return self.parse_accent_command(accent);
        }

        // Special characters: \ldots, \S, \ss, \quad, ...
        if let Some(special) = get_special_char(&cmd) {
            return Ok(Some(Inline::Text(special_char_to_string(special))));
        }

        // Formatting/layout commands with no Markdown equivalent
        if let Some((braced_args, optional_arg)) = noop_command_signature(&cmd) {
            self.consume_text_star();
            if optional_arg && matches!(self.current_token(), Token::LeftBracket) {
                let _ = self.parse_bracketed_text()?;
            }
            for _ in 0..braced_args {
                self.skip_whitespace_and_comments();
                if matches!(self.current_token(), Token::LeftBrace) {
                    let _ = self.parse_braced_text()?;
                }
            }
            return Ok(None);
        }

        if let Some(result) = self.parse_knowledge_command(&cmd)? {
            return Ok(result);
        }

        if let Some(literal) = text_symbol_command(&cmd) {
            return Ok(Some(Inline::Text(literal.to_string())));
        }

        match cmd.as_str() {
            "textbf" => {
                let content = self.parse_braced_inlines()?;
                Ok(Some(Inline::Strong(content)))
            }
            "textit" | "emph" | "textsl" => {
                let content = self.parse_braced_inlines()?;
                Ok(Some(Inline::Emph(content)))
            }
            "underline" | "underbar" | "ul" | "uline" => {
                let content = self.parse_braced_inlines()?;
                Ok(Some(Inline::Underline(content)))
            }
            "texttt" | "verb" | "code" => {
                let content = self.parse_braced_text()?;
                Ok(Some(Inline::Code(content)))
            }
            "ensuremath" => {
                let content = self.parse_braced_text()?;
                Ok(Some(Inline::InlineMath(content)))
            }
            "textsuperscript" => {
                let content = self.parse_braced_inlines()?;
                Ok(Some(Inline::Superscript(content)))
            }
            "textsubscript" => {
                let content = self.parse_braced_inlines()?;
                Ok(Some(Inline::Subscript(content)))
            }
            "textsc" => {
                let content = self.parse_braced_inlines()?;
                Ok(Some(Inline::SmallCaps(content)))
            }
            "href" => {
                // hyperref allows an options group: \href[options]{url}{text}
                if matches!(self.current_token(), Token::LeftBracket) {
                    let _ = self.parse_bracketed_text()?;
                }
                let url = self.parse_braced_text()?;
                let text = self.parse_braced_inlines()?;
                Ok(Some(Inline::Link { text, url, title: None }))
            }
            "url" => {
                let url = self.parse_braced_text()?;
                Ok(Some(Inline::Link {
                    text: vec![Inline::Text(url.clone())],
                    url,
                    title: None,
                }))
            }
            "hyperref" => {
                if matches!(self.current_token(), Token::LeftBracket) {
                    let _ = self.parse_bracketed_text()?;
                }
                let content = self.parse_braced_inlines()?;
                Ok(Some(Inline::Span {
                    attrs: HashMap::new(),
                    content,
                }))
            }
            "includegraphics" => {
                self.consume_text_star();
                if matches!(self.current_token(), Token::LeftBracket) {
                    let _ = self.parse_bracketed_text()?;
                }
                let path = self.parse_braced_text()?;
                Ok(Some(Inline::Image {
                    alt: Vec::new(),
                    url: path,
                    title: None,
                }))
            }

            // Content-preserving wrappers: color, boxes, size scaling
            "textcolor" => {
                if matches!(self.current_token(), Token::LeftBracket) {
                    let _ = self.parse_bracketed_text()?;
                }
                let _color = self.parse_braced_text()?;
                let content = self.parse_braced_inlines()?;
                Ok(Some(Inline::Span {
                    attrs: HashMap::new(),
                    content,
                }))
            }
            "colorbox" => {
                let _color = self.parse_braced_text()?;
                let content = self.parse_braced_inlines()?;
                Ok(Some(Inline::Span {
                    attrs: HashMap::new(),
                    content,
                }))
            }
            "mbox" | "hbox" | "fbox" | "text" | "textnormal" | "textrm" | "textsf" | "textup"
            | "textmd" | "centerline" | "makecell" | "smash" => {
                let content = self.parse_braced_inlines()?;
                Ok(Some(Inline::Span {
                    attrs: HashMap::new(),
                    content,
                }))
            }
            "makebox" | "framebox" => {
                while matches!(self.current_token(), Token::LeftBracket) {
                    let _ = self.parse_bracketed_text()?;
                }
                let content = self.parse_braced_inlines()?;
                Ok(Some(Inline::Span {
                    attrs: HashMap::new(),
                    content,
                }))
            }
            "parbox" => {
                if matches!(self.current_token(), Token::LeftBracket) {
                    let _ = self.parse_bracketed_text()?;
                }
                let _width = self.parse_braced_text()?;
                let content = self.parse_braced_inlines()?;
                Ok(Some(Inline::Span {
                    attrs: HashMap::new(),
                    content,
                }))
            }
            "raisebox" => {
                let _offset = self.parse_braced_text()?;
                while matches!(self.current_token(), Token::LeftBracket) {
                    let _ = self.parse_bracketed_text()?;
                }
                let content = self.parse_braced_inlines()?;
                Ok(Some(Inline::Span {
                    attrs: HashMap::new(),
                    content,
                }))
            }
            "scalebox" => {
                let _scale = self.parse_braced_text()?;
                if matches!(self.current_token(), Token::LeftBracket) {
                    let _ = self.parse_bracketed_text()?;
                }
                let content = self.parse_braced_inlines()?;
                Ok(Some(Inline::Span {
                    attrs: HashMap::new(),
                    content,
                }))
            }
            "resizebox" => {
                let _w = self.parse_braced_text()?;
                let _h = self.parse_braced_text()?;
                let content = self.parse_braced_inlines()?;
                Ok(Some(Inline::Span {
                    attrs: HashMap::new(),
                    content,
                }))
            }

            "enquote" => {
                let content = self.parse_braced_inlines()?;
                Ok(Some(Inline::Quoted {
                    quote_type: QuoteType::DoubleQuote,
                    content,
                }))
            }
            "caption" => {
                // Stray caption outside figure/table environments
                self.consume_text_star();
                if matches!(self.current_token(), Token::LeftBracket) {
                    let _ = self.parse_bracketed_text()?;
                }
                let content = self.parse_braced_inlines()?;
                Ok(Some(Inline::Strong(content)))
            }

            // Case conversion
            "MakeUppercase" | "MakeTextUppercase" | "uppercase" => {
                let text = self.parse_braced_text()?;
                Ok(Some(Inline::Text(text.to_uppercase())))
            }
            "MakeLowercase" | "MakeTextLowercase" | "lowercase" => {
                let text = self.parse_braced_text()?;
                Ok(Some(Inline::Text(text.to_lowercase())))
            }

            // Strikeout / highlight
            "st" | "sout" => {
                let content = self.parse_braced_inlines()?;
                Ok(Some(Inline::Strikeout(content)))
            }
            "hl" => {
                let content = self.parse_braced_inlines()?;
                Ok(Some(Inline::Span {
                    attrs: HashMap::new(),
                    content,
                }))
            }

            // Line breaks
            "newline" => Ok(Some(Inline::LineBreak)),
            "linebreak" => {
                if matches!(self.current_token(), Token::LeftBracket) {
                    let _ = self.parse_bracketed_text()?;
                }
                Ok(Some(Inline::LineBreak))
            }

            // \and (and NeurIPS-style \And/\AND) separate authors in \author{...}
            "and" | "And" | "AND" => Ok(Some(Inline::Text(String::from(", ")))),

            // \par ends the paragraph; as an inline (e.g. in fragments) it is a space
            "par" => Ok(Some(Inline::Space)),

            // Today
            "today" => {
                use chrono::Local;
                Ok(Some(Inline::Text(Local::now().format("%B %d, %Y").to_string())))
            }

            _ => {
                if self.command_registry.contains(&cmd) {
                    let mut args = Vec::new();
                    if matches!(self.current_token(), Token::LeftBrace) {
                        args.push(self.parse_braced_text()?);
                    }
                    if let Ok(result) = self.command_registry.handle(&cmd, &args) {
                        if !result.is_empty() && result != format!("\\{}", cmd) {
                            return Ok(Some(Inline::RawInline(result)));
                        }
                    }
                }
                self.parse_unknown_command(&cmd)
            }
        }
    }

    /// beamer `\begin{frame}<overlay>[options]{title}{subtitle} ... \end{frame}`:
    /// an unnumbered heading followed by the frame's content.
    fn parse_beamer_frame(&mut self) -> Result<Option<Block>> {
        self.skip_overlay_spec();
        self.skip_whitespace_and_comments();
        if matches!(self.current_token(), Token::LeftBracket) {
            let _ = self.collect_bracketed_tokens();
        }
        self.skip_overlay_spec();
        let mut title: Vec<Inline> = Vec::new();
        let mut subtitle: Vec<Inline> = Vec::new();
        self.skip_whitespace_only();
        if matches!(self.current_token(), Token::LeftBrace) {
            title = trim_inline_whitespace(self.parse_braced_inlines()?);
            self.skip_whitespace_only();
            if matches!(self.current_token(), Token::LeftBrace) {
                subtitle = trim_inline_whitespace(self.parse_braced_inlines()?);
            }
        }
        let mut blocks = Vec::new();
        while !self.is_at_end() {
            self.skip_whitespace_and_comments();
            match self.current_token() {
                Token::EndEnvironment(env) if env == "frame" => {
                    self.advance();
                    break;
                }
                Token::Command(cmd) if cmd == "frametitle" || cmd == "framesubtitle" => {
                    let is_title = cmd == "frametitle";
                    self.advance();
                    self.skip_overlay_spec();
                    if matches!(self.current_token(), Token::LeftBracket) {
                        let _ = self.collect_bracketed_tokens();
                    }
                    let content = trim_inline_whitespace(self.parse_braced_inlines()?);
                    if is_title {
                        title = content;
                    } else {
                        subtitle = content;
                    }
                }
                _ => {
                    if let Some(block) = self.parse_block()? {
                        blocks.push(block);
                    }
                }
            }
        }
        let mut out = Vec::new();
        if !title.is_empty() {
            out.push(Block::Section {
                level: 2,
                title,
                label: None,
            });
        }
        if !subtitle.is_empty() {
            out.push(Block::Paragraph(vec![Inline::Emph(subtitle)]));
        }
        out.extend(blocks);
        Ok(match out.len() {
            0 => None,
            1 => out.pop(),
            _ => Some(Block::Composite(out)),
        })
    }

    /// beamer `\begin{block}{Title}`: a titled quotation.
    fn parse_beamer_block(&mut self, env_name: &str) -> Result<Option<Block>> {
        self.skip_overlay_spec();
        self.skip_whitespace_only();
        let title = if matches!(self.current_token(), Token::LeftBrace) {
            trim_inline_whitespace(self.parse_braced_inlines()?)
        } else {
            Vec::new()
        };
        self.skip_overlay_spec();
        let mut blocks = Vec::new();
        if !title.is_empty() {
            blocks.push(Block::Paragraph(vec![Inline::Strong(title)]));
        }
        while !self.is_at_end() {
            self.skip_whitespace_and_comments();
            if matches!(self.current_token(), Token::EndEnvironment(env) if env == env_name) {
                self.advance();
                break;
            }
            if let Some(block) = self.parse_block()? {
                blocks.push(block);
            }
        }
        Ok(Some(Block::Quote(blocks)))
    }

    /// Parse blocks up to `\end{env_name}`.
    fn parse_environment_blocks(&mut self, env_name: &str) -> Result<Vec<Block>> {
        let mut blocks = Vec::new();
        while !self.is_at_end() {
            self.skip_whitespace_and_comments();
            if matches!(self.current_token(), Token::EndEnvironment(env) if env == env_name) {
                self.advance();
                break;
            }
            if let Some(block) = self.parse_block()? {
                blocks.push(block);
            }
        }
        Ok(blocks)
    }

    /// The content of an environment as one run of inlines.
    fn read_environment_inlines(&mut self, env_name: &str) -> Result<Vec<Inline>> {
        let mut inlines = Vec::new();
        while !self.is_at_end() {
            match self.current_token() {
                Token::EndEnvironment(env) if env == env_name => {
                    self.advance();
                    break;
                }
                Token::ParBreak => {
                    self.advance();
                    inlines.push(Inline::Space);
                }
                _ => {
                    if let Some(inline) = self.parse_inline()? {
                        inlines.push(inline);
                    }
                }
            }
        }
        Ok(trim_inline_whitespace(inlines))
    }

    /// Skip a beamer overlay specification (`<2->`, `<+->`, `<1,3>`).
    fn skip_overlay_spec(&mut self) {
        if let Token::Text(text) = self.current_token() {
            if is_overlay_spec(text) {
                self.advance();
            }
        }
    }

    /// Commands described by the knowledge tables plus assorted content
    /// commands. Returns `Ok(Some(result))` when handled.
    fn parse_knowledge_command(&mut self, cmd: &str) -> Result<Option<Option<Inline>>> {
        if let Some(result) = self.parse_glossary_command(cmd)? {
            return Ok(Some(result));
        }
        if let Some(signature) = knowledge::setup_command(cmd) {
            self.consume_text_star();
            self.consume_signature(signature)?;
            // Commands whose trailing groups vary: consume them all
            if matches!(cmd, "titleformat" | "newcolumntype" | "AddToHook" | "AddToHookNext"
                | "setbeamertemplate" | "setbeamercolor" | "setbeamerfont"
                | "newlist" | "renewlist" | "DeclareFieldFormat")
            {
                self.consume_trailing_groups(8)?;
            }
            return Ok(Some(None));
        }
        if let Some((operands, value)) = knowledge::tex_assignment(cmd) {
            self.consume_tex_assignment(operands, value);
            return Ok(Some(None));
        }
        if let Some(markup) = knowledge::doc_markup(cmd) {
            self.skip_whitespace_only();
            if !matches!(self.current_token(), Token::LeftBrace) {
                // used without an argument: not doc markup after all
                return Ok(Some(None));
            }
            let text = self.parse_braced_text()?;
            let text = text.trim().to_string();
            return Ok(Some(Some(match markup {
                knowledge::DocMarkup::Code => Inline::Code(text),
                knowledge::DocMarkup::ControlSequence => {
                    Inline::Code(format!("\\{}", text.trim_start_matches('\\')))
                }
                knowledge::DocMarkup::Meta(open, close) => {
                    Inline::Text(format!("{}{}{}", open, text, close))
                }
            })));
        }
        if let Some(text) = self.parse_siunitx(cmd)? {
            return Ok(Some(Some(Inline::Text(text))));
        }
        match cmd {
            // beamer overlays: content is shown (as on the final slide)
            "only" | "uncover" | "visible" | "invisible" | "onslide" | "temporal"
            | "alt" | "action" | "alert" | "structure" | "pause" | "note" | "frame"
            | "againframe" | "titlepage" | "framezoom" | "hyperlinkslidenext"
            | "hyperlinkslideprev" | "transdissolve" | "transblindshorizontal"
            | "transboxin" | "transfade" | "transduration" => {
                self.skip_overlay_spec();
                match cmd {
                    "pause" | "transdissolve" | "transblindshorizontal" | "transboxin"
                    | "transfade" | "transduration" | "hyperlinkslidenext"
                    | "hyperlinkslideprev" => {
                        if matches!(self.current_token(), Token::LeftBracket) {
                            let _ = self.collect_bracketed_tokens();
                        }
                        return Ok(Some(None));
                    }
                    "onslide" => {
                        // \onslide<2-> as a declaration, or \onslide<2>{text}
                        self.skip_overlay_spec();
                        if !matches!(self.current_token(), Token::LeftBrace) {
                            return Ok(Some(None));
                        }
                    }
                    "note" => {
                        if matches!(self.current_token(), Token::LeftBracket) {
                            let _ = self.collect_bracketed_tokens();
                        }
                        let _ = self.collect_braced_tokens();
                        return Ok(Some(None));
                    }
                    "againframe" | "framezoom" => {
                        self.consume_trailing_groups(3)?;
                        return Ok(Some(None));
                    }
                    "titlepage" => {
                        // The title block, as \maketitle renders it
                        self.push_front(vec![Token::Command(String::from("maketitle"))]);
                        return Ok(Some(None));
                    }
                    _ => {}
                }
                if matches!(self.current_token(), Token::LeftBracket) {
                    let _ = self.collect_bracketed_tokens();
                }
                self.skip_overlay_spec();
                let content = self.parse_braced_inlines()?;
                if cmd == "alt" || cmd == "temporal" {
                    // \alt<spec>{shown}{otherwise}: keep the first
                    self.skip_whitespace_only();
                    let _ = self.collect_braced_tokens();
                    if cmd == "temporal" {
                        let _ = self.collect_braced_tokens();
                    }
                }
                if cmd == "invisible" {
                    return Ok(Some(None));
                }
                let content = trim_inline_whitespace(content);
                if content.is_empty() {
                    return Ok(Some(None));
                }
                return Ok(Some(Some(if matches!(cmd, "alert" | "structure") {
                    Inline::Strong(content)
                } else {
                    span_or_single(content)
                })));
            }
            "tnote" => {
                let content = self.parse_braced_inlines()?;
                return Ok(Some(Some(Inline::Superscript(content))));
            }
            "keywords" | "keyword" | "IEEEkeywords" | "kwd" | "terms" => {
                self.skip_whitespace_only();
                if !matches!(self.current_token(), Token::LeftBrace) {
                    return Ok(None);
                }
                let content = trim_inline_whitespace(self.parse_braced_inlines()?);
                let mut line = vec![Inline::Strong(vec![Inline::Text(String::from("Keywords:"))])];
                if !content.is_empty() {
                    line.push(Inline::Space);
                    line.extend(content);
                }
                return Ok(Some(Some(span_or_single(line))));
            }
            "texorpdfstring" => {
                let content = self.parse_braced_inlines()?;
                self.skip_whitespace_and_comments();
                let _ = self.parse_braced_text()?;
                return Ok(Some(Some(span_or_single(content))));
            }
            "hypertarget" => {
                let _ = self.parse_braced_text()?;
                let content = self.parse_braced_inlines()?;
                return Ok(Some(Some(span_or_single(content))));
            }
            "hyperlink" => {
                let target = self.parse_braced_text()?;
                let text = self.parse_braced_inlines()?;
                return Ok(Some(Some(Inline::Link {
                    text,
                    url: format!("#{}", target.trim()),
                    title: None,
                })));
            }
            "epigraph" => {
                let quote = self.parse_braced_inlines()?;
                self.skip_whitespace_and_comments();
                let source = self.parse_braced_inlines()?;
                let mut content = vec![Inline::Emph(quote)];
                if !source.is_empty() {
                    content.push(Inline::Text(String::from(" \u{2014} ")));
                    content.extend(source);
                }
                return Ok(Some(Some(span_or_single(content))));
            }
            "hologo" | "Hologo" => {
                let name = self.parse_braced_text()?;
                return Ok(Some(Some(Inline::Text(name))));
            }
            "space" | "nobreakspace" | "enspace" | "thinspace" | "enskip" => {
                return Ok(Some(Some(Inline::Text(String::from(" ")))));
            }
            "char" => {
                // \char65, \char"41
                let value = self.read_char_code();
                return Ok(Some(value.map(|c| Inline::Text(c.to_string()))));
            }
            "bye" | "endinput" | "stop" => {
                // End of input: of the current \input file, or of the document
                self.skip_to_end_of_file();
                return Ok(Some(None));
            }
            "texed@endfile" => return Ok(Some(None)),
            "comment" => {
                // \comment ... \endcomment (comment package, plain form)
                while !self.is_at_end() {
                    let done = matches!(self.current_token(), Token::Command(c) if c == "endcomment");
                    self.advance();
                    if done {
                        break;
                    }
                }
                return Ok(Some(None));
            }
            "endcomment" => return Ok(Some(None)),
            "declaretheorem" => {
                self.parse_declaretheorem()?;
                return Ok(Some(None));
            }
            "newmdtheoremenv" | "newtcbtheorem" | "newtheoremrep" => {
                // \newmdtheoremenv[opts]{name}[counter]{Title}[within]
                let unnumbered = self.consume_text_star();
                self.skip_whitespace_and_comments();
                if matches!(self.current_token(), Token::LeftBracket) {
                    let _ = self.parse_bracketed_text()?;
                }
                self.parse_newtheorem_command(unnumbered)?;
                if cmd == "newtcbtheorem" {
                    self.consume_trailing_groups(2)?;
                }
                return Ok(Some(None));
            }
            _ => {}
        }
        if let Some(symbol) = knowledge::text_math_symbol(cmd) {
            return Ok(Some(Some(Inline::Text(symbol.to_string()))));
        }
        Ok(None)
    }

    /// siunitx commands, rendered with Unicode units and superscripts.
    fn parse_siunitx(&mut self, cmd: &str) -> Result<Option<String>> {
        let kind = match cmd {
            "num" | "si" | "unit" | "SI" | "qty" | "ang" | "SIrange" | "qtyrange" | "numrange"
            | "SIlist" | "qtylist" | "numlist" | "numproduct" | "qtyproduct" | "SIproduct"
            | "tablenum" | "complexnum" | "complexqty" | "qtyrange*" => cmd,
            _ => return Ok(None),
        };
        // Only treat as siunitx when an argument follows (\num could be a
        // user's own command name elsewhere)
        let mut look = 0;
        while matches!(self.peek_at(look), Some(Token::Whitespace(_))) {
            look += 1;
        }
        if !matches!(self.peek_at(look), Some(Token::LeftBrace) | Some(Token::LeftBracket)) {
            return Ok(None);
        }
        let arg = |parser: &mut Parser| -> Result<String> {
            parser.skip_whitespace_only();
            while matches!(parser.current_token(), Token::LeftBracket) {
                let _ = parser.collect_bracketed_tokens();
                parser.skip_whitespace_only();
            }
            parser.parse_braced_text()
        };
        let nbsp = '\u{00A0}';
        let list = |items: &[String]| -> String {
            match items.len() {
                0 => String::new(),
                1 => items[0].clone(),
                _ => format!(
                    "{} and {}",
                    items[..items.len() - 1].join(", "),
                    items[items.len() - 1]
                ),
            }
        };
        Ok(Some(match kind {
            "num" | "tablenum" | "complexnum" => siunitx::format_number(&arg(self)?),
            "si" | "unit" => siunitx::format_unit(&arg(self)?),
            "SI" | "qty" | "complexqty" => {
                let number = siunitx::format_number(&arg(self)?);
                let unit = siunitx::format_unit(&arg(self)?);
                if unit.starts_with('°') || unit == "%" || unit.starts_with('′') || unit.starts_with('″') {
                    format!("{}{}", number, unit)
                } else {
                    format!("{}{}{}", number, nbsp, unit)
                }
            }
            "ang" => {
                let spec = arg(self)?;
                let parts: Vec<&str> = spec.split(';').collect();
                let marks = ["°", "′", "″"];
                parts
                    .iter()
                    .zip(marks.iter())
                    .filter(|(p, _)| !p.trim().is_empty())
                    .map(|(p, m)| format!("{}{}", siunitx::format_number(p), m))
                    .collect::<String>()
            }
            "SIrange" | "qtyrange" | "qtyrange*" => {
                let a = siunitx::format_number(&arg(self)?);
                let b = siunitx::format_number(&arg(self)?);
                let unit = siunitx::format_unit(&arg(self)?);
                format!("{a}{nbsp}{unit} to {b}{nbsp}{unit}")
            }
            "numrange" => {
                let a = siunitx::format_number(&arg(self)?);
                let b = siunitx::format_number(&arg(self)?);
                format!("{} to {}", a, b)
            }
            "SIlist" | "qtylist" => {
                let items: Vec<String> = arg(self)?.split(';').map(siunitx::format_number).collect();
                let unit = siunitx::format_unit(&arg(self)?);
                let items: Vec<String> = items.into_iter().map(|n| format!("{}{}{}", n, nbsp, unit)).collect();
                list(&items)
            }
            "numlist" => {
                let items: Vec<String> = arg(self)?.split(';').map(siunitx::format_number).collect();
                list(&items)
            }
            "numproduct" | "qtyproduct" | "SIproduct" => {
                let spec = arg(self)?;
                let product = spec
                    .split('x')
                    .map(siunitx::format_number)
                    .collect::<Vec<_>>()
                    .join(" × ");
                if kind == "numproduct" {
                    product
                } else {
                    let unit = siunitx::format_unit(&arg(self)?);
                    format!("{}{}{}", product, nbsp, unit)
                }
            }
            _ => return Ok(None),
        }))
    }

    /// Consume the arguments of a set-up command.
    fn consume_signature(&mut self, signature: knowledge::Signature) -> Result<()> {
        if signature.leading_optional {
            self.skip_whitespace_only();
            while matches!(self.current_token(), Token::LeftBracket) {
                let _ = self.collect_bracketed_tokens();
                self.skip_whitespace_only();
            }
        }
        for _ in 0..signature.braced {
            self.skip_whitespace_only();
            match self.current_token() {
                Token::LeftBrace => {
                    let _ = self.collect_braced_tokens();
                }
                // A single-token argument (\setlength\parindent{0pt})
                Token::Command(_) | Token::Text(_) => self.advance(),
                _ => break,
            }
        }
        if signature.trailing_optional {
            while matches!(self.current_token(), Token::LeftBracket) {
                let _ = self.collect_bracketed_tokens();
            }
        }
        Ok(())
    }

    /// Consume up to `max` directly following `[...]`/`{...}` groups.
    fn consume_trailing_groups(&mut self, max: usize) -> Result<()> {
        for _ in 0..max {
            let mut look = 0;
            while matches!(
                self.peek_at(look),
                Some(Token::Whitespace(_)) | Some(Token::Newline) | Some(Token::Comment(_))
            ) {
                look += 1;
            }
            match self.peek_at(look) {
                Some(Token::LeftBrace) => {
                    self.skip_n(look);
                    let _ = self.collect_braced_tokens();
                }
                Some(Token::LeftBracket) => {
                    self.skip_n(look);
                    let _ = self.collect_bracketed_tokens();
                }
                _ => break,
            }
        }
        Ok(())
    }

    /// Consume a TeX assignment: `operands` control sequences, then (when
    /// `value`) an optional `=` and a number/dimension/glue specification.
    fn consume_tex_assignment(&mut self, operands: usize, value: bool) {
        for _ in 0..operands {
            self.skip_whitespace_only();
            if matches!(self.current_token(), Token::Command(_)) {
                self.advance();
            }
        }
        if !value {
            return;
        }
        let mut consumed = 0;
        let mut seen_value = false;
        while consumed < 24 {
            match self.current_token().clone() {
                Token::Whitespace(_) | Token::Newline => self.advance(),
                Token::Text(t) if is_tex_value_text(&t) => {
                    seen_value |= t.chars().any(|c| c.is_ascii_digit());
                    self.advance();
                }
                // `\catcode`\@=11`: the backquote lexes as a quote character
                Token::Text(t) if t == "\u{2018}" || t == "`" => self.advance(),
                Token::Command(name)
                    if !seen_value
                        || matches!(
                            name.as_str(),
                            "fill" | "fil" | "z@" | "p@" | "@ne" | "m@ne" | "tw@" | "thr@@"
                                | "sixt@@n" | "@m" | "@M" | "@MM"
                        ) =>
                {
                    self.advance();
                    // a register as the value: \parskip=\baselineskip
                    seen_value = true;
                }
                _ => break,
            }
            consumed += 1;
        }
    }

    /// Read the character code after `\char`.
    fn read_char_code(&mut self) -> Option<char> {
        self.skip_whitespace_only();
        match self.current_token().clone() {
            Token::Text(t) => {
                self.advance();
                let (radix, digits) = if let Some(rest) = t.strip_prefix('"') {
                    (16, rest)
                } else if let Some(rest) = t.strip_prefix('\'') {
                    (8, rest)
                } else {
                    (10, t.as_str())
                };
                let numeric: String = digits.chars().take_while(|c| c.is_digit(radix)).collect();
                let rest = digits[numeric.len()..].to_string();
                if !rest.is_empty() {
                    self.push_front(vec![Token::Text(rest)]);
                }
                u32::from_str_radix(&numeric, radix).ok().and_then(char::from_u32)
            }
            _ => None,
        }
    }

    /// `\endinput`/`\bye`: skip to the end of the current input file
    /// (marked by `texed@endfile`), or of the whole document.
    fn skip_to_end_of_file(&mut self) {
        while !self.is_at_end() {
            let at_marker = matches!(self.current_token(), Token::Command(c) if c == "texed@endfile");
            self.advance();
            if at_marker {
                return;
            }
        }
    }

    /// thmtools: `\declaretheorem[name=Theorem, numberwithin=section]{thm}`.
    fn parse_declaretheorem(&mut self) -> Result<()> {
        self.skip_whitespace_and_comments();
        let options = if matches!(self.current_token(), Token::LeftBracket) {
            self.parse_bracketed_text()?
        } else {
            String::new()
        };
        self.skip_whitespace_and_comments();
        let names = self.parse_braced_text()?;
        // Options may also follow the name (newer thmtools syntax)
        let options = if matches!(self.current_token(), Token::LeftBracket) {
            format!("{},{}", options, self.parse_bracketed_text()?)
        } else {
            options
        };
        let option = |key: &str| -> Option<String> {
            options.split(',').find_map(|kv| {
                let mut parts = kv.splitn(2, '=');
                (parts.next()?.trim() == key).then(|| parts.next().unwrap_or("").trim().to_string())
            })
        };
        let numbered = !matches!(option("numbered").as_deref(), Some("no"));
        let sibling = option("sibling")
            .or_else(|| option("numberlike"))
            .or_else(|| option("sharenumber"));
        let within = option("numberwithin").or_else(|| option("parent"));
        for name in names.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            let display = option("name").or_else(|| option("title")).unwrap_or_else(|| {
                let mut chars = name.chars();
                chars
                    .next()
                    .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
                    .unwrap_or_default()
            });
            self.state.define_theorem_env(
                name.to_string(),
                display,
                numbered,
                sibling.clone(),
                within.clone(),
            );
        }
        Ok(())
    }

    /// Fallback for commands with no known meaning: the command itself is
    /// dropped (reported as a warning), its optional arguments are skipped
    /// and the text of its braced arguments is kept, so content wrapped in
    /// custom markup (`\acrshort{gpu}`, `\mynote{...}`) is not lost.
    fn parse_unknown_command(&mut self, cmd: &str) -> Result<Option<Inline>> {
        if !self.in_preamble {
            self.state.warn(format!(
                "unknown command `\\{}` dropped (arguments kept as text)",
                cmd
            ));
        }
        self.consume_text_star();
        let mut parts: Vec<Inline> = Vec::new();
        for _ in 0..9 {
            match self.current_token() {
                Token::LeftBracket => {
                    let _ = self.collect_bracketed_tokens();
                }
                Token::LeftBrace => {
                    let content = self.parse_braced_inlines()?;
                    let content = trim_inline_whitespace(content);
                    if !content.is_empty() {
                        if !parts.is_empty() {
                            parts.push(Inline::Space);
                        }
                        parts.extend(content);
                    }
                }
                _ => break,
            }
        }
        Ok(match parts.len() {
            0 => None,
            1 => parts.pop(),
            _ => Some(Inline::Span {
                attrs: HashMap::new(),
                content: parts,
            }),
        })
    }

    /// Apply an accent command to its argument (braced group or next character).
    fn parse_accent_command(
        &mut self,
        accent: crate::lexer_extended::AccentType,
    ) -> Result<Option<Inline>> {
        match self.current_token().clone() {
            Token::LeftBrace => {
                let text = self.parse_braced_text()?;
                let mut chars = text.chars();
                match chars.next() {
                    Some(base) => {
                        let accented = apply_accent_string(&accent, base);
                        let rest: String = chars.collect();
                        Ok(Some(Inline::Text(format!("{}{}", accented, rest))))
                    }
                    None => Ok(Some(Inline::Text(standalone_accent_text(&accent)))),
                }
            }
            Token::Text(text) => {
                self.advance();
                let mut chars = text.chars();
                match chars.next() {
                    Some(base) => {
                        let accented = apply_accent_string(&accent, base);
                        let rest: String = chars.collect();
                        Ok(Some(Inline::Text(format!("{}{}", accented, rest))))
                    }
                    None => Ok(Some(Inline::Text(standalone_accent_text(&accent)))),
                }
            }
            _ => Ok(Some(Inline::Text(standalone_accent_text(&accent)))),
        }
    }
    
    fn is_citation_command(&self, cmd: &str) -> bool {
        matches!(cmd, "cite" | "citep" | "citet" | "citealt" | "citealp" | 
                      "citeauthor" | "citeyear" | "citeyearpar" |
                      "autocite" | "textcite" | "parencite" | "footcite")
    }
    
    fn is_reference_command(&self, cmd: &str) -> bool {
        matches!(
            cmd,
            "ref" | "eqref" | "autoref" | "Autoref" | "nameref" | "Nameref" | "pageref" | "cref"
                | "Cref" | "vref" | "Vref" | "labelcref" | "cpageref" | "Cpageref"
                | "namecref" | "nameCref" | "lcnamecref" | "namecrefs" | "nameCrefs"
                | "lcnamecrefs" | "subref" | "labelcpageref" | "vpageref" | "zref"
                | "cref*" | "Cref*" | "autoref*" | "ref*" | "fref" | "Fref" | "prettyref"
        )
    }
    
    fn parse_citation_command(&mut self, cmd: &str) -> Result<Option<Inline>> {
        // Optional note arguments precede the keys: \citep[postnote]{keys}
        // or \citep[prenote][postnote]{keys}
        let mut prenote = None;
        let mut postnote = None;
        if matches!(self.current_token(), Token::LeftBracket) {
            let first = self.parse_bracketed_text()?;
            if matches!(self.current_token(), Token::LeftBracket) {
                prenote = Some(first);
                postnote = Some(self.parse_bracketed_text()?);
            } else {
                // A single optional argument is the postnote
                postnote = Some(first);
            }
        }
        // Trim separators the citation formatter adds itself; resolve TeX ties
        let clean_note = |note: String| {
            let note = note
                .replace('~', "\u{00A0}")
                .trim()
                .trim_end_matches(',')
                .trim_end()
                .to_string();
            (!note.is_empty()).then_some(note)
        };
        let prenote = prenote.and_then(clean_note);
        let postnote = postnote.and_then(clean_note);

        let args_text = self.parse_braced_text()?;
        let (keys, _, _) = parse_citation_args(&format!("{{{}}}", args_text));

        Ok(Some(Inline::Cite {
            citations: keys,
            mode: citation_mode_for(cmd),
            prenote,
            postnote,
            content: Vec::new(),
        }))
    }
    
    fn parse_reference_command(&mut self, cmd: &str) -> Result<Option<Inline>> {
        self.consume_text_star();
        self.skip_whitespace_only();
        if matches!(self.current_token(), Token::LeftBracket) {
            let _ = self.parse_bracketed_text()?;
        }
        let label = self.parse_braced_text()?;
        let kind = match cmd {
            "fref" | "vref" | "prettyref" | "zref" => "cref",
            "Fref" | "Vref" => "Cref",
            "subref" | "vpageref" => "ref",
            other => other,
        };
        Ok(Some(self.render_reference(kind, &label)))
    }

    /// Render a \ref-family command against the label table. `labels` may
    /// be a comma-separated list (cleveref). Unresolved labels stay as
    /// `Inline::Ref` so the post-parse pass can resolve forward references.
    fn render_reference(&self, kind: &str, labels: &str) -> Inline {
        let keys: Vec<&str> = labels.split(',').map(str::trim).filter(|l| !l.is_empty()).collect();
        let infos: Vec<&crate::state::LabelInfo> = keys.iter().filter_map(|k| self.state.get_label(k)).collect();
        if keys.is_empty() || infos.len() != keys.len() {
            return Inline::Ref {
                kind: kind.to_string(),
                label: labels.to_string(),
            };
        }

        let number_of = |info: &crate::state::LabelInfo| info.number.clone();
        let text = match kind {
            "nameref" | "Nameref" => infos[0].title.clone().unwrap_or_else(|| keys[0].to_string()),
            "eqref" => infos
                .iter()
                .map(|i| format!("({})", number_of(i)))
                .collect::<Vec<_>>()
                .join(", "),
            "namecref" | "nameCref" | "lcnamecref" | "namecrefs" | "nameCrefs" | "lcnamecrefs" => {
                let capital = kind.starts_with("nameC");
                let plural = kind.ends_with('s');
                ref_type_name(infos[0], capital, plural, RefStyle::Cleveref).0
            }
            "cref" | "Cref" | "vref" | "Vref" | "cpageref" | "Cpageref" | "autoref" | "Autoref" => {
                let style = if kind.ends_with("autoref") || kind == "Autoref" {
                    RefStyle::Hyperref
                } else {
                    RefStyle::Cleveref
                };
                let capital = kind.starts_with(|c: char| c.is_uppercase()) || style == RefStyle::Hyperref;
                // Group consecutive references of the same type
                let mut groups: Vec<(String, bool, Vec<String>)> = Vec::new();
                for info in &infos {
                    let (singular, parens) = ref_type_name(info, false, false, style);
                    match groups.last_mut() {
                        Some((name, _, numbers)) if *name == singular => numbers.push(number_of(info)),
                        _ => groups.push((singular, parens, vec![number_of(info)])),
                    }
                }
                let rendered: Vec<String> = groups
                    .iter()
                    .enumerate()
                    .map(|(gi, (_, parens, numbers))| {
                        let info = infos
                            .iter()
                            .find(|i| ref_type_name(i, false, false, style).0 == groups[gi].0)
                            .unwrap();
                        let (name, _) = ref_type_name(info, capital && gi == 0, numbers.len() > 1, style);
                        let numbers: Vec<String> = numbers
                            .iter()
                            .map(|n| if *parens { format!("({})", n) } else { n.clone() })
                            .collect();
                        let list = join_and(&numbers);
                        if list.is_empty() {
                            name
                        } else if name.is_empty() {
                            list
                        } else {
                            format!("{} {}", name, list)
                        }
                    })
                    .collect();
                join_and(&rendered)
            }
            // \ref, \pageref, \labelcref, ... : the number(s) only
            _ => join_and(&infos.iter().map(|i| number_of(i)).collect::<Vec<_>>()),
        };
        Inline::Text(text)
    }

    fn parse_macro_expansion(&mut self, cmd: &str) -> Result<Option<Inline>> {
        let args = self.parse_macro_arguments(cmd)?;

        if let Some(expanded) = self.macro_processor.expand_macro(&self.state, cmd, &args) {
            if !self.consume_expansion_bytes(cmd, expanded.len()) || !self.consume_expansion_budget(cmd) {
                return Ok(None);
            }
            // Re-lex the expansion and splice it into the token stream so the
            // body is fully parsed (formatting, math, nested macros, ...).
            self.push_front(tex::lex_fragment(&expanded));
            Ok(None)
        } else {
            Ok(Some(Inline::Text(format!("\\{}", cmd))))
        }
    }

    fn parse_braced_group(&mut self) -> Result<Option<Inline>> {
        self.advance(); // consume '{'

        // Font declarations at group start style the whole group: {\bf text}
        let mut styles = Vec::new();
        loop {
            let mut lookahead = 0;
            while matches!(self.peek_at(lookahead), Some(Token::Whitespace(_))) {
                lookahead += 1;
            }
            match self.peek_at(lookahead) {
                Some(Token::Command(cmd)) => match declaration_style(cmd) {
                    Some(style) => {
                        self.skip_n(lookahead + 1);
                        styles.push(style);
                    }
                    None => break,
                },
                _ => break,
            }
        }

        let inlines = self.parse_inlines_until(Token::RightBrace)?;

        if matches!(self.current_token(), Token::RightBrace) {
            self.advance();
        }

        let mut inlines = if styles.is_empty() {
            inlines
        } else {
            trim_inline_whitespace(inlines)
        };

        for style in styles.iter().rev() {
            if inlines.is_empty() {
                break;
            }
            inlines = apply_decl_style(style, inlines);
        }

        // Return as a span or flatten
        if inlines.len() == 1 {
            Ok(Some(inlines[0].clone()))
        } else if inlines.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Inline::Span {
                attrs: HashMap::new(),
                content: inlines,
            }))
        }
    }

    fn parse_braced_inlines(&mut self) -> Result<Vec<Inline>> {
        if !matches!(self.current_token(), Token::LeftBrace) {
            return Ok(vec![]);
        }

        self.advance(); // consume '{'
        
        // Don't skip whitespace - preserve it in the content
        let inlines = self.parse_inlines_until(Token::RightBrace)?;
        
        if matches!(self.current_token(), Token::RightBrace) {
            self.advance();
        }

        Ok(inlines)
    }

    fn parse_braced_text(&mut self) -> Result<String> {
        let tokens = self.collect_braced_tokens();
        let mut text = String::new();
        for token in &tokens {
            text.push_str(&self.token_to_string(token));
        }
        Ok(text)
    }

    fn parse_bracketed_text(&mut self) -> Result<String> {
        if !matches!(self.current_token(), Token::LeftBracket) {
            return Ok(String::new());
        }

        self.advance(); // consume '['
        let text = self.read_until_token(Token::RightBracket);

        if matches!(self.current_token(), Token::RightBracket) {
            self.advance();
        }

        Ok(text)
    }

    fn consume_text_star(&mut self) -> bool {
        if matches!(self.current_token(), Token::Text(text) if text == "*") {
            self.advance();
            true
        } else {
            false
        }
    }

    fn parse_macro_definition_command(&mut self, mode: MacroDefinitionMode) -> Result<()> {
        let tex_style = std::mem::take(&mut self.macro_def_tex_style);
        self.skip_whitespace_and_comments();
        // Both \newcommand{\name} and the unbraced \newcommand\name are valid
        let name_raw = if matches!(self.current_token(), Token::LeftBrace) {
            self.parse_braced_text()?
        } else if let Token::Command(name) = self.current_token() {
            let name = name.clone();
            self.advance();
            name
        } else if let Token::Ref { kind, .. } | Token::Cite { kind, .. } = self.current_token() {
            // \renewcommand\ref... is lexed specially; recover the name
            let name = kind.clone();
            self.advance();
            name
        } else {
            String::new()
        };
        if name_raw.is_empty() {
            return Ok(());
        }

        let mut num_params = 0;
        let mut optional_param = None;
        let mut delimiters = Vec::new();
        let mut prefix = String::new();

        if tex_style {
            // TeX parameter text: everything up to the body's opening brace,
            // e.g. \def\pair#1#2{...} or delimited \def\foo#1.#2\par{...}
            let mut param_text = String::new();
            let mut guard = 0;
            while !self.is_at_end()
                && !matches!(self.current_token(), Token::LeftBrace | Token::ParBreak)
                && guard < 64
            {
                param_text.push_str(&self.token_to_string(self.current_token()));
                self.advance();
                guard += 1;
            }
            let (p, d, n) = parse_tex_param_text(&param_text);
            prefix = p;
            delimiters = d;
            num_params = n;
        } else {
            self.skip_whitespace_and_comments();
            if matches!(self.current_token(), Token::LeftBracket) {
                num_params = self.parse_bracketed_text()?.trim().parse::<usize>().unwrap_or(0).min(9);
            }
            self.skip_whitespace_and_comments();
            if matches!(self.current_token(), Token::LeftBracket) {
                optional_param = Some(self.parse_bracketed_text()?);
            }
            self.skip_whitespace_and_comments();
        }

        let body = if matches!(self.current_token(), Token::LeftBrace) {
            self.parse_braced_text()?
        } else {
            // \newcommand\foo\bar: a single-token body
            match self.current_token().clone() {
                Token::Command(c) => {
                    self.advance();
                    format!("\\{}", c)
                }
                _ => String::new(),
            }
        };
        let name = name_raw.trim().trim_start_matches('\\');

        if !name.is_empty() {
            let macro_def = MacroDef {
                num_params,
                body,
                optional_param,
                delimiters,
                prefix,
            };
            match mode {
                MacroDefinitionMode::New | MacroDefinitionMode::Renew => {
                    self.state.aliases.remove(name);
                    self.state.macros.insert(name.to_string(), macro_def);
                }
                MacroDefinitionMode::Provide => {
                    self.state.macros.entry(name.to_string()).or_insert(macro_def);
                }
            }
        }

        Ok(())
    }

    /// `\DeclareMathOperator{\argmax}{arg\,max}` (star: limits form).
    fn parse_declare_math_operator(&mut self) -> Result<()> {
        // A leading * was consumed by the caller via consume_text_star; check
        // the raw token again in case of `\DeclareMathOperator*`
        self.skip_whitespace_and_comments();
        let starred = self.consume_text_star();
        self.skip_whitespace_and_comments();
        let name = if matches!(self.current_token(), Token::LeftBrace) {
            self.parse_braced_text()?
        } else if let Token::Command(name) = self.current_token().clone() {
            self.advance();
            name
        } else {
            return Ok(());
        };
        self.skip_whitespace_and_comments();
        let text = self.parse_braced_text()?;
        let name = name.trim().trim_start_matches('\\').to_string();
        if !name.is_empty() {
            let op = if starred { "\\operatorname*" } else { "\\operatorname" };
            self.state.macros.insert(
                name,
                MacroDef {
                    body: format!("{}{{{}}}", op, text),
                    ..Default::default()
                },
            );
        }
        Ok(())
    }

    /// xparse: `\NewDocumentCommand{\foo}{m o m}{body}`.
    fn parse_xparse_definition(&mut self) -> Result<()> {
        self.skip_whitespace_and_comments();
        let name = if matches!(self.current_token(), Token::LeftBrace) {
            self.parse_braced_text()?
        } else if let Token::Command(name) = self.current_token().clone() {
            self.advance();
            name
        } else {
            return Ok(());
        };
        self.skip_whitespace_and_comments();
        let spec = self.parse_braced_text()?;
        self.skip_whitespace_and_comments();
        let body = self.parse_braced_text()?;

        // Count argument specifiers; O{default}/D..{default} groups skipped
        let mut num_params = 0;
        let mut optional_param = None;
        let mut chars = spec.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                'm' | 'r' | 'R' | 'v' | 'b' => num_params += 1,
                'o' | 'd' | 's' | 't' => {
                    if num_params == 0 && optional_param.is_none() && ch == 'o' {
                        optional_param = Some(String::new());
                    }
                    num_params += 1;
                }
                'O' | 'D' => {
                    let mut default = String::new();
                    if chars.peek() == Some(&'{') {
                        chars.next();
                        let mut depth = 1;
                        for c in chars.by_ref() {
                            match c {
                                '{' => depth += 1,
                                '}' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                            default.push(c);
                        }
                    }
                    if num_params == 0 && optional_param.is_none() {
                        optional_param = Some(default);
                    }
                    num_params += 1;
                }
                _ => {}
            }
        }
        let name = name.trim().trim_start_matches('\\').to_string();
        if !name.is_empty() {
            self.state.aliases.remove(&name);
            self.state.macros.insert(
                name,
                MacroDef {
                    num_params: num_params.min(9),
                    body,
                    optional_param,
                    ..Default::default()
                },
            );
        }
        Ok(())
    }

    fn parse_newtheorem_command(&mut self, unnumbered: bool) -> Result<()> {
        let env_name = self.parse_braced_text()?;
        if env_name.is_empty() {
            return Ok(());
        }

        let shared_counter = if matches!(self.current_token(), Token::LeftBracket) {
            let counter = self.parse_bracketed_text()?;
            (!counter.is_empty()).then_some(counter)
        } else {
            None
        };

        let display_name = self.parse_braced_text()?;

        let within = if matches!(self.current_token(), Token::LeftBracket) {
            let scope = self.parse_bracketed_text()?;
            (!scope.is_empty()).then_some(scope)
        } else {
            None
        };

        self.state.define_theorem_env(
            env_name,
            if display_name.is_empty() {
                "Theorem".to_string()
            } else {
                display_name
            },
            !unnumbered,
            shared_counter,
            within,
        );

        Ok(())
    }

    fn inlines_to_plain_text(&self, inlines: &[Inline]) -> String {
        let mut text = String::new();
        for inline in inlines {
            match inline {
                Inline::Text(value)
                | Inline::Code(value)
                | Inline::InlineMath(value)
                | Inline::RawInline(value) => {
                    text.push_str(value);
                }
                Inline::Ref { label, .. } => text.push_str(label),
                Inline::Space | Inline::SoftBreak | Inline::LineBreak => text.push(' '),
                Inline::Emph(content)
                | Inline::Strong(content)
                | Inline::Strikeout(content)
                | Inline::Underline(content)
                | Inline::Superscript(content)
                | Inline::Subscript(content)
                | Inline::SmallCaps(content) => text.push_str(&self.inlines_to_plain_text(content)),
                Inline::Link { text: content, .. }
                | Inline::Image { alt: content, .. }
                | Inline::Cite { content, .. }
                | Inline::Span { content, .. }
                | Inline::Quoted { content, .. } => text.push_str(&self.inlines_to_plain_text(content)),
                Inline::Note(_) => {}
            }
        }
        text.trim().to_string()
    }

    fn parse_macro_arguments(&mut self, cmd: &str) -> Result<Vec<String>> {
        let Some(def) = self.macro_processor.get_definition(&self.state, cmd) else {
            return Ok(Vec::new());
        };
        let num_params = def.num_params;
        let optional_param = def.optional_param.clone();
        let delimiters = def.delimiters.clone();
        let prefix = def.prefix.clone();

        // Literal text required between the name and the first parameter
        if !prefix.trim().is_empty() {
            let _ = self.collect_delimited_argument(prefix.trim());
        }

        let mut args = Vec::new();
        let required_params = if optional_param.is_some() {
            self.skip_whitespace_only();
            if matches!(self.current_token(), Token::LeftBracket) {
                args.push(self.parse_bracketed_text()?);
            } else if let Some(default) = optional_param {
                args.push(default);
            }
            num_params.saturating_sub(1)
        } else {
            num_params
        };

        for i in 0..required_params {
            let index = args.len();
            let delimiter = delimiters.get(index).cloned().unwrap_or_default();
            if !delimiter.trim().is_empty() {
                args.push(self.collect_delimited_argument(delimiter.trim()));
                continue;
            }
            let _ = i;
            args.push(self.parse_undelimited_argument()?);
        }

        Ok(args)
    }

    /// Skip spaces and comments but not paragraph breaks or newlines that
    /// precede a blank line (TeX skips spaces before undelimited arguments).
    fn skip_whitespace_only(&mut self) {
        while matches!(
            self.current_token(),
            Token::Whitespace(_) | Token::Newline | Token::Comment(_)
        ) {
            self.advance();
        }
    }

    /// One undelimited TeX macro argument: a braced group, or else a single
    /// token (a single character of a text run, as in TeX).
    fn parse_undelimited_argument(&mut self) -> Result<String> {
        self.skip_whitespace_only();
        match self.current_token().clone() {
            Token::LeftBrace => self.parse_braced_text(),
            Token::Text(text) => {
                self.advance();
                let mut chars = text.chars();
                let first = chars.next().map(String::from).unwrap_or_default();
                let rest: String = chars.collect();
                if !rest.is_empty() {
                    self.push_front(vec![Token::Text(rest)]);
                }
                Ok(first)
            }
            Token::RightBrace | Token::EndEnvironment(_) | Token::ParBreak | Token::Eof => {
                Ok(String::new())
            }
            other => {
                self.advance();
                Ok(self.token_to_string(&other))
            }
        }
    }

    /// Collect tokens up to `delimiter` (at brace depth 0), consuming the
    /// delimiter. A single surrounding brace pair is stripped, as in TeX.
    fn collect_delimited_argument(&mut self, delimiter: &str) -> String {
        let is_par = delimiter == "\\par";
        let mut collected = String::new();
        let mut depth = 0usize;
        let mut guard = 0;
        while !self.is_at_end() && guard < 20_000 {
            guard += 1;
            let token = self.current_token().clone();
            if depth == 0 && is_par && matches!(token, Token::ParBreak) {
                self.advance();
                break;
            }
            match token {
                Token::LeftBrace => depth += 1,
                Token::RightBrace => {
                    if depth == 0 {
                        break; // end of enclosing group: argument ends here
                    }
                    depth -= 1;
                }
                _ => {}
            }
            self.advance();
            let piece = self.token_to_string(&token);
            let mut search_from = collected.len().saturating_sub(delimiter.len());
            while !collected.is_char_boundary(search_from) {
                search_from -= 1;
            }
            collected.push_str(&piece);
            if depth == 0 && !matches!(token, Token::LeftBrace | Token::RightBrace) {
                if let Some(pos) = collected[search_from..].find(delimiter) {
                    let pos = search_from + pos;
                    let leftover = collected[pos + delimiter.len()..].to_string();
                    collected.truncate(pos);
                    if !leftover.is_empty() {
                        self.push_front(tex::lex_fragment(&leftover));
                    }
                    break;
                }
            }
        }
        // Spaces after the macro name are skipped by TeX's tokenizer
        let collected = collected.trim_start().to_string();
        let trimmed = collected.trim();
        if trimmed.starts_with('{') && trimmed.ends_with('}') && balanced_outer_group(trimmed) {
            trimmed[1..trimmed.len() - 1].to_string()
        } else {
            collected
        }
    }

    /// Lex an `\input`ed file and splice its tokens at the cursor.
    fn splice_input_file(&mut self, cmd: &str) -> Result<()> {
        self.skip_whitespace_and_comments();
        if cmd == "import" {
            // \import{dir/}{file}
            let dir = self.parse_braced_text()?;
            self.skip_whitespace_and_comments();
            let file = self.parse_braced_text()?;
            let path = format!("{}{}", dir.trim(), file.trim());
            return self.splice_named_file(cmd, &path);
        }
        let filename = if matches!(self.current_token(), Token::LeftBrace) {
            self.parse_braced_text()?
        } else {
            // TeX-style \input file (terminated by whitespace)
            match self.current_token().clone() {
                Token::Text(t) => {
                    self.advance();
                    let mut name = t;
                    while let Token::Text(more) = self.current_token().clone() {
                        name.push_str(&more);
                        self.advance();
                    }
                    name
                }
                _ => return Ok(()),
            }
        };
        self.splice_named_file(cmd, filename.trim())
    }

    fn splice_named_file(&mut self, cmd: &str, filename: &str) -> Result<()> {
        if filename.is_empty() {
            return Ok(());
        }
        let base = self.base_path.as_deref();
        let content = if cmd == "include" {
            self.include_system.include_file(&mut self.state, filename, base)
        } else {
            self.include_system.input_file(&mut self.state, filename, base)
        };
        match content {
            Ok(content) => {
                if self.consume_expansion_budget(&format!("input:{}", filename)) {
                    let at_letter = filename.ends_with(".sty") || filename.ends_with(".cls");
                    let mut tokens = Lexer::new(&content)
                        .with_at_letter(at_letter)
                        .tokenize()
                        .unwrap_or_default();
                    if matches!(tokens.last(), Some(Token::Eof)) {
                        tokens.pop();
                    }
                    tokens.push(Token::Command(String::from("texed@endfile")));
                    self.account_source_tokens(&tokens);
                    self.push_front(tokens);
                }
            }
            Err(_) => {
                self.state.warn(format!(
                    "could not resolve \\{}{{{}}} — content skipped",
                    cmd, filename
                ));
            }
        }
        Ok(())
    }

    /// Expand user-defined macros inside math content, so constructs like
    /// \newcommand{\R}{\mathbb{R}} render in Markdown math (KaTeX/MathJax
    /// don't know document macros).
    fn expand_macros_in_math(&mut self, content: &str) -> String {
        let expanded = self.expand_math_with_engine(content, true);
        math::clean_math(&expanded)
    }

    /// Parse a string of LaTeX as a sequence of inlines, sharing all parser
    /// state (macros, labels, footnotes, citations). Any \label inside the
    /// fragment is extracted and returned separately.
    fn parse_inline_fragment(&mut self, content: &str) -> Result<(Vec<Inline>, Option<String>)> {
        let mut lexer = Lexer::new(content);
        let tokens = lexer.tokenize()?;
        self.parse_inline_fragment_tokens(tokens)
    }

    /// Like `parse_inline_fragment`, but starting from tokens. The current
    /// token stream is swapped out, the fragment is parsed to exhaustion, and
    /// the original stream is restored.
    fn parse_inline_fragment_tokens(
        &mut self,
        mut tokens: Vec<Token>,
    ) -> Result<(Vec<Inline>, Option<String>)> {
        let mut label = None;
        tokens.retain(|token| match token {
            Token::Label(l) => {
                label = Some(l.clone());
                false
            }
            _ => true,
        });
        if !matches!(tokens.last(), Some(Token::Eof)) {
            tokens.push(Token::Eof);
        }

        let saved_tokens = std::mem::replace(&mut self.tokens, reverse_tokens(tokens));
        let saved_expansions = std::mem::take(&mut self.active_expansions);

        let mut inlines = Vec::new();
        let mut result = Ok(());
        while !self.is_at_end() {
            match self.current_token() {
                Token::ParBreak => {
                    self.advance();
                    inlines.push(Inline::Space);
                }
                _ => match self.parse_inline() {
                    Ok(Some(inline)) => inlines.push(inline),
                    Ok(None) => {}
                    Err(e) => {
                        result = Err(e);
                        break;
                    }
                },
            }
        }

        self.tokens = saved_tokens;
        self.active_expansions = saved_expansions;
        result?;

        Ok((inlines, label))
    }

    /// Collect the raw tokens of a braced group (brace-aware), consuming it.
    fn collect_braced_tokens(&mut self) -> Vec<Token> {
        if !matches!(self.current_token(), Token::LeftBrace) {
            return Vec::new();
        }
        self.advance(); // consume '{'

        let mut depth = 1usize;
        let mut tokens = Vec::new();
        while !self.is_at_end() {
            match self.current_token() {
                Token::LeftBrace => {
                    depth += 1;
                    tokens.push(Token::LeftBrace);
                }
                Token::RightBrace => {
                    depth -= 1;
                    if depth == 0 {
                        self.advance();
                        break;
                    }
                    tokens.push(Token::RightBrace);
                }
                token => tokens.push(token.clone()),
            }
            self.advance();
        }
        tokens
    }

    fn parse_included_content(&mut self, filename: &str, content: &str) -> Result<Option<Block>> {
        let mut lexer = Lexer::new(content);
        let tokens = lexer.tokenize()?;
        let mut child = Parser::new(tokens);

        child.metadata = std::mem::take(&mut self.metadata);
        child.state = std::mem::take(&mut self.state);
        child.citation_manager = std::mem::replace(
            &mut self.citation_manager,
            CitationManager::new(crate::state::CitationStyle::Numeric),
        );
        child.macro_processor = std::mem::take(&mut self.macro_processor);
        child.include_system = std::mem::take(&mut self.include_system);
        child.command_registry = std::mem::take(&mut self.command_registry);
        child.footnote_inlines = std::mem::take(&mut self.footnote_inlines);
        child.depth = self.depth;
        child.in_preamble = self.in_preamble;
        child.macro_expansions = self.macro_expansions;
        child.macro_budget = self.macro_budget.saturating_add(child.macro_budget);
        child.expanded_bytes = self.expanded_bytes;
        child.source_token_count = self.source_token_count.saturating_add(child.source_token_count);
        for (name, count) in &self.source_command_counts {
            *child.source_command_counts.entry(name.clone()).or_insert(0) += count;
        }

        let include_base = self
            .include_system
            .get_resolved_path(filename, self.base_path.as_deref())
            .and_then(|path| path.parent().map(|p| p.to_path_buf()))
            .or_else(|| self.base_path.clone());
        if let Some(base_path) = include_base {
            child = child.with_base_path(base_path);
        }

        let parsed = child.parse_blocks();

        self.macro_expansions = child.macro_expansions;
        self.macro_budget = child.macro_budget;
        self.expanded_bytes = child.expanded_bytes;
        self.source_token_count = child.source_token_count;
        self.source_command_counts = std::mem::take(&mut child.source_command_counts);
        self.in_preamble = child.in_preamble;
        if child.document_ended {
            self.tokens.clear();
            self.document_ended = true;
        }
        self.metadata = child.metadata;
        self.state = child.state;
        self.citation_manager = child.citation_manager;
        self.macro_processor = child.macro_processor;
        self.include_system = child.include_system;
        self.command_registry = child.command_registry;
        self.footnote_inlines = child.footnote_inlines;
        let blocks = parsed?;

        if blocks.is_empty() {
            Ok(None)
        } else if blocks.len() == 1 {
            Ok(Some(blocks.into_iter().next().unwrap()))
        } else {
            Ok(Some(Block::Composite(blocks)))
        }
    }

    fn read_until_end_environment(&mut self, env_name: &str) -> Result<String> {
        let mut content = String::new();

        while !self.is_at_end() {
            if let Token::EndEnvironment(env) = self.current_token() {
                if env == env_name {
                    self.advance();
                    break;
                }
            }

            content.push_str(&self.token_to_string(self.current_token()));
            self.advance();
        }

        Ok(content)
    }

    fn read_until_token(&mut self, end_token: Token) -> String {
        let mut content = String::new();

        while !self.is_at_end() && self.current_token() != &end_token {
            content.push_str(&self.token_to_string(self.current_token()));
            self.advance();
        }

        content
    }

    fn token_to_string(&self, token: &Token) -> String {
        match token {
            Token::Text(t) => t.clone(),
            Token::Whitespace(w) => w.clone(),
            Token::Newline => String::from("\n"),
            Token::ParBreak => String::from("\n\n"),
            Token::Command(cmd) => format!("\\{}", cmd),
            Token::LeftBrace => String::from("{"),
            Token::RightBrace => String::from("}"),
            Token::LeftBracket => String::from("["),
            Token::RightBracket => String::from("]"),
            Token::Backslash => String::from("\\\\"),
            Token::Ampersand => String::from("&"),
            Token::Tilde => String::from("~"),
            Token::InlineMath(m) => format!("${}$", m),
            Token::DisplayMath(m) => format!("$${}$$", m),
            Token::BeginEnvironment(env) => format!("\\begin{{{}}}", env),
            Token::EndEnvironment(env) => format!("\\end{{{}}}", env),
            Token::VerbatimEnv { name, content } | Token::MathEnv { name, content } => {
                format!("\\begin{{{}}}{}\\end{{{}}}", name, content, name)
            }
            Token::Verb(v) => format!("\\verb|{}|", v),
            Token::Item => String::from("\\item "),
            Token::Label(l) => format!("\\label{{{}}}", l),
            Token::Ref { kind, label } => format!("\\{}{{{}}}", kind, label),
            Token::Cite { kind, keys } => format!("\\{}{{{}}}", kind, keys),
            Token::Section(level, title, flags) => {
                let name = match level {
                    0 if flags.part => "part",
                    0 => "chapter",
                    1 => "section",
                    2 => "subsection",
                    3 => "subsubsection",
                    4 => "paragraph",
                    _ => "subparagraph",
                };
                format!("\\{}{{{}}}", name, title)
            }
            Token::Comment(_) | Token::Percent | Token::Eof => String::new(),
        }
    }

    fn parse_command_block(&mut self, cmd: String) -> Result<Option<Block>> {
        match cmd.as_str() {
            // Document structure (skip these)
            "documentclass" | "documentstyle" => {
                self.advance();
                self.skip_whitespace_and_comments();
                // Class options [11pt,a4paper]
                let options = self.parse_bracketed_text()?;
                self.skip_whitespace_and_comments();
                let class = self.parse_braced_text()?;
                let class = class.trim();
                if matches!(
                    class,
                    "book" | "report" | "memoir" | "scrbook" | "scrreprt" | "amsbook" | "thesis"
                        | "ucthesis" | "mitthesis" | "jsbook" | "jreport" | "tufte-book"
                        | "octavo" | "suthesis-2e" | "kaobook"
                ) || class.ends_with("thesis")
                {
                    self.state.has_chapters = true;
                }
                // LaTeX 2.09: \documentstyle[amssymb,epsf]{article} lists
                // packages as options
                if cmd == "documentstyle" {
                    for package in options.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                        let package = package.to_string();
                        self.load_local_package(&package);
                    }
                }
                // A local .cls next to the document carries its macros
                if !class.is_empty() {
                    let class_file = format!("{}.cls", class);
                    self.load_local_package(&class_file);
                }
                Ok(None)
            }
            
            // Paragraph control
            "par" | "noindent" => {
                self.advance();
                Ok(None)
            }
            
            // Horizontal rules
            "hline" | "hrule" => {
                self.advance();
                Ok(Some(Block::HorizontalRule))
            }
            
            // Include commands
            "include" | "input" => {
                self.advance();
                let filename = self.parse_braced_text()?;
                if !filename.is_empty() {
                    // Try to include the file
                    let base = self.base_path.as_deref();
                    match if cmd == "include" {
                        self.include_system.include_file(&mut self.state, &filename, base)
                    } else {
                        self.include_system.input_file(&mut self.state, &filename, base)
                    } {
                        Ok(content) => self.parse_included_content(&filename, &content),
                        Err(_) => {
                            self.state.warn(format!(
                                "could not resolve \\{}{{{}}} — content skipped",
                                cmd, filename
                            ));
                            Ok(None)
                        }
                    }
                } else {
                    Ok(None)
                }
            }
            
            // Package loading
            "usepackage" | "RequirePackage" => {
                self.advance();
                self.skip_whitespace_and_comments();
                let options = if matches!(self.current_token(), Token::LeftBracket) {
                    self.parse_bracketed_text()?
                } else {
                    String::new()
                };
                self.skip_whitespace_and_comments();
                let packages = self.parse_braced_text()?;
                // Optional [date] after the package list
                if matches!(self.current_token(), Token::LeftBracket) {
                    let _ = self.parse_bracketed_text()?;
                }
                for package in packages.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                    let package = package.to_string();
                    // biblatex declares its citation style as a package option
                    if package == "biblatex" {
                        let author_year = options.split(',').any(|opt| {
                            let mut parts = opt.splitn(2, '=');
                            let key = parts.next().unwrap_or("").trim();
                            let value = parts.next().unwrap_or("").trim();
                            key == "style" && value.contains("authoryear")
                        });
                        if author_year {
                            self.citation_manager
                                .set_style(crate::state::CitationStyle::AuthorYear);
                        }
                    }
                    self.load_local_package(&package);
                }
                Ok(None)
            }
            
            // Macro definitions
            "newcommand" | "renewcommand" | "providecommand" | "def" | "gdef" | "edef"
            | "xdef" | "DeclareRobustCommand" | "newrobustcmd" | "renewrobustcmd"
            | "providerobustcmd" | "DeclareMathOperator" | "DeclareTextFontCommand"
            | "NewDocumentCommand" | "RenewDocumentCommand" | "ProvideDocumentCommand"
            | "DeclareDocumentCommand" => {
                self.advance();
                self.consume_text_star();
                if cmd == "DeclareMathOperator" {
                    self.parse_declare_math_operator()?;
                    return Ok(None);
                }
                if cmd.ends_with("DocumentCommand") {
                    self.parse_xparse_definition()?;
                    return Ok(None);
                }
                let tex_style = matches!(cmd.as_str(), "def" | "gdef" | "edef" | "xdef");
                self.macro_def_tex_style = tex_style;
                let mode = match cmd.as_str() {
                    "renewcommand" | "def" | "gdef" | "edef" | "xdef" | "renewrobustcmd"
                    | "DeclareRobustCommand" | "DeclareTextFontCommand" => MacroDefinitionMode::Renew,
                    "providecommand" => MacroDefinitionMode::Provide,
                    _ => MacroDefinitionMode::New,
                };
                self.parse_macro_definition_command(mode)?;
                Ok(None)
            }
            
            // Environment definitions
            "newenvironment" | "renewenvironment" => {
                self.advance();
                self.consume_text_star();
                let name = self.parse_braced_text()?;
                let num_params = if matches!(self.current_token(), Token::LeftBracket) {
                    self.parse_bracketed_text()?.trim().parse::<usize>().unwrap_or(0)
                } else {
                    0
                };
                let optional_param = if matches!(self.current_token(), Token::LeftBracket) {
                    Some(self.parse_bracketed_text()?)
                } else {
                    None
                };
                let begin_def = self.parse_braced_text()?;
                let end_def = self.parse_braced_text()?;
                if !name.is_empty() {
                    self.state.environments.insert(
                        name,
                        crate::state::EnvironmentDef {
                            num_params,
                            optional_param,
                            begin_def,
                            end_def,
                        },
                    );
                }
                Ok(None)
            }

            // Theorem-like definitions
            "newtheorem" => {
                self.advance();
                let unnumbered = self.consume_text_star();
                self.parse_newtheorem_command(unnumbered)?;
                Ok(None)
            }
            
            // Bibliography
            "bibliography" | "addbibresource" => {
                self.advance();
                let bibfile = self.parse_braced_text()?;
                if !bibfile.is_empty() {
                    self.state.bibliography.push(bibfile);
                }
                Ok(None)
            }
            
            "bibliographystyle" => {
                self.advance();
                let style = self.parse_braced_text()?;
                if let Some(citation_style) = citation_style_for(&style) {
                    self.citation_manager.set_style(citation_style);
                }
                Ok(None)
            }
            
            // Metadata commands
            "title" | "author" | "date" => {
                self.advance();
                if matches!(self.current_token(), Token::LeftBracket) {
                    let _ = self.parse_bracketed_text()?;
                }
                let tokens = self.collect_braced_tokens();
                let (inlines, _) = self.parse_inline_fragment_tokens(tokens)?;
                let rendered =
                    crate::converter::MarkdownConverter::render_inlines_fragment(&inlines);
                let value = rendered.split_whitespace().collect::<Vec<_>>().join(" ");
                // \and between authors renders as ", ": tidy the joins
                let value = if cmd == "author" {
                    value
                        .replace(" ,", ",")
                        .replace(",,", ",")
                        .trim()
                        .trim_start_matches(',')
                        .trim_end_matches([',', ' '])
                        .trim()
                        .to_string()
                } else {
                    value.trim().to_string()
                };
                if !value.is_empty() {
                    self.metadata.insert(cmd.clone(), value);
                }
                Ok(None)
            }

            "maketitle" => {
                self.advance();
                let mut blocks = Vec::new();
                if let Some(title) = self.metadata.get("title") {
                    blocks.push(Block::Section {
                        level: 0,
                        title: vec![Inline::RawInline(title.clone())],
                        label: None,
                    });
                }
                let mut byline = String::new();
                if let Some(author) = self.metadata.get("author") {
                    byline.push_str(author);
                }
                if let Some(date) = self.metadata.get("date") {
                    if !byline.is_empty() {
                        byline.push_str("  \n");
                    }
                    byline.push_str(date);
                }
                if !byline.is_empty() {
                    blocks.push(Block::Paragraph(vec![Inline::Emph(vec![Inline::RawInline(
                        byline,
                    )])]));
                }
                match blocks.len() {
                    0 => Ok(None),
                    1 => Ok(Some(blocks.into_iter().next().unwrap())),
                    _ => Ok(Some(Block::Composite(blocks))),
                }
            }
            
            // Graphics path
            "graphicspath" => {
                self.advance();
                let paths = self.parse_braced_text()?;
                if !paths.is_empty() {
                    let graphics_paths = crate::include_system::parse_graphics_path(&paths);
                    for path in graphics_paths {
                        self.include_system.add_search_path(path);
                    }
                }
                Ok(None)
            }
            
            // Toggle commands
            "newtoggle" => {
                self.advance();
                let name = self.parse_braced_text()?;
                if !name.is_empty() {
                    self.state.set_toggle(name, false);
                }
                Ok(None)
            }
            "toggletrue" => {
                self.advance();
                let name = self.parse_braced_text()?;
                if !name.is_empty() {
                    self.state.set_toggle(name, true);
                }
                Ok(None)
            }
            "togglefalse" => {
                self.advance();
                let name = self.parse_braced_text()?;
                if !name.is_empty() {
                    self.state.set_toggle(name, false);
                }
                Ok(None)
            }
            
            _ => {
                self.advance();
                // Consume any arguments the command declares so they don't
                // leak into the output as stray text.
                if let Some((braced_args, optional_arg)) = noop_command_signature(&cmd) {
                    self.consume_text_star();
                    if optional_arg && matches!(self.current_token(), Token::LeftBracket) {
                        let _ = self.parse_bracketed_text()?;
                    }
                    for _ in 0..braced_args {
                        self.skip_whitespace_and_comments();
                        if matches!(self.current_token(), Token::LeftBrace) {
                            let _ = self.parse_braced_text()?;
                        }
                    }
                }
                Ok(None)
            }
        }
    }

    /// Read a package/class file found next to the document (local style
    /// files hold the document's macro and theorem definitions) and splice
    /// it in. Packages not found locally are standard ones: nothing to do.
    fn load_local_package(&mut self, package: &str) {
        let base = self.base_path.clone();
        if let Ok(Some(content)) =
            self.include_system
                .load_package(&mut self.state, package, &[], base.as_deref())
        {
            if !self.consume_expansion_bytes(package, content.len()) {
                return;
            }
            let mut tokens = Lexer::new(&content)
                .with_at_letter(true)
                .tokenize()
                .unwrap_or_default();
            if matches!(tokens.last(), Some(Token::Eof)) {
                tokens.pop();
            }
            // Package code is set-up only, even if it contains stray text
            tokens.push(Token::Command(String::from("texed@endpackage")));
            self.account_source_tokens(&tokens);
            let was_preamble = self.in_preamble;
            self.in_preamble = true;
            self.package_preamble_restore.push(was_preamble);
            self.push_front(tokens);
        }
    }

    /// Tokens read from another source file count as source: they grow the
    /// expansion budgets like the main file's tokens do.
    fn account_source_tokens(&mut self, tokens: &[Token]) {
        self.source_token_count = self.source_token_count.saturating_add(tokens.len());
        self.macro_budget = self
            .macro_budget
            .saturating_add(tokens.len().saturating_mul(4));
        for token in tokens {
            if let Token::Command(name) = token {
                *self.source_command_counts.entry(name.clone()).or_insert(0) += 1;
            }
        }
    }

    /// Charge `bytes` of expansion output against the document's budget.
    fn consume_expansion_bytes(&mut self, name: &str, bytes: usize) -> bool {
        let limit = BASE_EXPANSION_BYTES.saturating_add(self.source_token_count.saturating_mul(1024));
        self.expanded_bytes = self.expanded_bytes.saturating_add(bytes);
        if self.expanded_bytes > limit {
            if self.refused_macros.insert(name.to_string()) {
                self.state.warn(format!(
                    "macro `\\{}` expands to unbounded output — expansion stopped",
                    name
                ));
            }
            return false;
        }
        true
    }

    /// Current value of a named LaTeX counter.
    fn counter_value(&self, name: &str) -> usize {
        match name {
            "chapter" => self.state.section_counters[0],
            "section" => self.state.section_counters[1],
            "subsection" => self.state.section_counters[2],
            "subsubsection" => self.state.section_counters[3],
            "paragraph" => self.state.section_counters[4],
            "subparagraph" => self.state.section_counters[5],
            "equation" => self.state.equation_counter,
            "figure" => self.state.figure_counter,
            "table" => self.state.table_counter,
            "footnote" => self.state.footnote_counter,
            _ => 0,
        }
    }

    fn skip_whitespace_and_comments(&mut self) {
        while !self.is_at_end() {
            match self.current_token() {
                Token::Whitespace(_) | Token::Comment(_) => {
                    self.advance();
                }
                _ => break,
            }
        }
    }

    fn current_token(&self) -> &Token {
        self.tokens.last().unwrap_or(&EOF_TOKEN)
    }

    /// The token `k` positions ahead of the cursor (0 = current).
    fn peek_at(&self, k: usize) -> Option<&Token> {
        let len = self.tokens.len();
        if k < len {
            self.tokens.get(len - 1 - k)
        } else {
            None
        }
    }

    /// Discard the next `n` tokens.
    fn skip_n(&mut self, n: usize) {
        let len = self.tokens.len();
        let n = n.min(len);
        self.tokens.truncate(len - n);
        self.consumed += n as u64;
    }

    /// Insert tokens so that `tokens[0]` becomes the current token.
    fn push_front(&mut self, tokens: Vec<Token>) {
        self.tokens
            .extend(tokens.into_iter().rev().filter(|t| !matches!(t, Token::Eof)));
    }

    fn advance(&mut self) {
        if self.tokens.pop().is_some() {
            self.consumed += 1;
        }
    }

    fn is_at_end(&self) -> bool {
        matches!(self.current_token(), Token::Eof)
    }
}

static EOF_TOKEN: Token = Token::Eof;

/// Reverse a lexed token sequence into the parser's stack order, dropping
/// end-of-input markers (end of input is simply an empty stack).
fn reverse_tokens(tokens: Vec<Token>) -> Vec<Token> {
    let mut tokens: Vec<Token> = tokens
        .into_iter()
        .filter(|t| !matches!(t, Token::Eof))
        .collect();
    tokens.reverse();
    tokens
}

/// Map a BibTeX style name to a citation rendering style. Returns None for
/// unknown styles (keep the current default).
fn citation_style_for(style: &str) -> Option<crate::state::CitationStyle> {
    use crate::state::CitationStyle;
    let style = style.trim().to_lowercase();
    match style.as_str() {
        "plain" | "abbrv" | "unsrt" | "alpha" | "ieeetr" | "acm" | "siam" | "unsrtnat" => {
            Some(CitationStyle::Numeric)
        }
        "plainnat" | "abbrvnat" | "apalike" | "apa" | "chicago" | "agsm" | "harvard"
        | "authordate1" | "authordate2" | "authordate3" | "authordate4" | "kluwer"
        | "newapa" | "named" => Some(CitationStyle::AuthorYear),
        _ => None,
    }
}

/// Map a \cite-family command name to its formatting mode.
fn citation_mode_for(cmd: &str) -> CitationMode {
    match cmd {
        "citet" | "textcite" => CitationMode::Textual,
        "citep" | "parencite" => CitationMode::Parenthetical,
        "citeauthor" => CitationMode::Author,
        "citeyear" => CitationMode::Year,
        "citeyearpar" => CitationMode::YearPar,
        "citealt" => CitationMode::Alt,
        "citealp" => CitationMode::Alp,
        _ => CitationMode::Normal,
    }
}

/// Commands that only affect print layout and produce no Markdown output.
/// Returns (number of braced args, whether an optional [arg] may precede them).
fn noop_command_signature(cmd: &str) -> Option<(usize, bool)> {
    match cmd {
        // Zero-argument layout/font declarations
        "centering" | "raggedright" | "raggedleft" | "noindent" | "indent" | "newpage"
        | "clearpage" | "cleardoublepage" | "samepage" | "protect" | "relax"
        | "ignorespaces" | "strut" | "hfill" | "vfill" | "hss" | "break" | "nobreak"
        | "allowbreak" | "frenchspacing" | "nonfrenchspacing" | "sloppy" | "fussy"
        | "selectfont" | "normalfont" | "em" | "bfseries" | "mdseries" | "itshape"
        | "upshape" | "slshape" | "scshape" | "rmfamily" | "sffamily" | "ttfamily"
        | "bf" | "it" | "tt" | "sc" | "sl" | "rm" | "sf" | "cal" | "tiny" | "scriptsize"
        | "footnotesize" | "small" | "normalsize" | "large" | "Large" | "LARGE" | "huge"
        | "Huge" | "appendix" | "frontmatter" | "mainmatter" | "backmatter"
        | "tableofcontents" | "listoffigures" | "listoftables" | "printindex"
        | "makeatletter" | "makeatother" | "onecolumn" | "flushbottom" | "raggedbottom"
        | "qedhere" | "smallbreak" | "medbreak" | "bigbreak" | "smallskip" | "medskip"
        | "bigskip" | "leavevmode" | "unskip" | "doublespacing" | "singlespacing"
        | "onehalfspacing" | "endinput" | "hrulefill" | "dotfill" | "checkmark"
        | "newblock" | "boldmath" | "unboldmath" | "ignorespacesafterend" => {
            Some((0, false))
        }

        // Optional-argument only
        "pagebreak" | "nopagebreak" | "printbibliography" | "twocolumn" | "item" => {
            Some((0, true))
        }

        // One braced argument
        "vspace" | "hspace" | "vskip" | "hskip" | "phantom" | "hphantom" | "vphantom"
        | "pagenumbering" | "thispagestyle" | "pagestyle" | "linespread" | "nocite"
        | "hyphenation" | "markright" | "enlargethispage" | "setstretch" | "index"
        | "glossary" | "hypersetup" | "geometry" | "usetikzlibrary" | "color"
        | "pagecolor" | "institute" | "titlerunning" | "authorrunning" | "acmConference"
        | "subjclass" | "PARstart" | "captionsetup" | "floatplacement" => {
            Some((1, true))
        }

        // Two braced arguments
        "setlength" | "addtolength" | "setcounter" | "addtocounter" | "numberwithin"
        | "markboth" | "fontsize" | "DeclareMathOperator" | "settowidth"
        | "addcontentsline" | "PassOptionsToPackage" => Some((2, false)),

        // Three braced arguments
        "definecolor" | "addtocontents" => Some((3, false)),

        "fancyhead" | "fancyfoot" | "setmainfont" | "setmonofont" => Some((1, true)),

        "rule" => Some((2, true)),

        _ => None,
    }
}

/// Text-mode symbol commands mapped straight to Unicode.
fn text_symbol_command(cmd: &str) -> Option<&'static str> {
    Some(match cmd {
        "textbackslash" => "\\",
        "textasciitilde" => "~",
        "textasciicircum" => "^",
        "textasciigrave" => "`",
        "textasciiacute" => "\u{00B4}",
        "textasciidieresis" => "\u{00A8}",
        "textquotedbl" => "\"",
        "textquotesingle" => "'",
        "textbraceleft" => "{",
        "textbraceright" => "}",
        "textdollar" => "$",
        "textnumbersign" => "#",
        "textpercent" => "%",
        "textampersand" => "&",
        "textasteriskcentered" => "*",
        "textbullet" => "\u{2022}",
        "textonehalf" => "\u{00BD}",
        "textonequarter" => "\u{00BC}",
        "textthreequarters" => "\u{00BE}",
        "textordfeminine" => "\u{00AA}",
        "textordmasculine" => "\u{00BA}",
        "textlangle" => "\u{27E8}",
        "textrangle" => "\u{27E9}",
        "textleftarrow" => "\u{2190}",
        "textrightarrow" => "\u{2192}",
        "textuparrow" => "\u{2191}",
        "textdownarrow" => "\u{2193}",
        "textcelsius" => "\u{2103}",
        "textbaht" => "\u{0E3F}",
        "textwon" => "\u{20A9}",
        "textnaira" => "\u{20A6}",
        "textlira" => "\u{20A4}",
        "textrecipe" => "\u{211E}",
        "textestimated" => "\u{212E}",
        "textinterrobang" => "\u{203D}",
        "textreferencemark" => "\u{203B}",
        "textmusicalnote" => "\u{266A}",
        "textbar" => "|",
        "textless" => "<",
        "textgreater" => ">",
        "textunderscore" => "_",
        "textemdash" => "\u{2014}",
        "textendash" => "\u{2013}",
        "textellipsis" => "\u{2026}",
        "textquotedblleft" => "\u{201C}",
        "textquotedblright" => "\u{201D}",
        "textquoteleft" => "\u{2018}",
        "textquoteright" => "\u{2019}",
        "textdegree" => "\u{00B0}",
        "texttimes" => "\u{00D7}",
        "textpm" => "\u{00B1}",
        "textminus" => "\u{2212}",
        "textcent" => "\u{00A2}",
        "textyen" => "\u{00A5}",
        "textsterling" => "\u{00A3}",
        "texteuro" => "\u{20AC}",
        "textcopyright" => "\u{00A9}",
        "textregistered" => "\u{00AE}",
        "texttrademark" => "\u{2122}",
        "textsection" => "\u{00A7}",
        "textparagraph" => "\u{00B6}",
        "textdagger" => "\u{2020}",
        "textdaggerdbl" => "\u{2021}",
        "textperiodcentered" => "\u{00B7}",
        "textvisiblespace" => "\u{2423}",
        "textexclamdown" => "\u{00A1}",
        "textquestiondown" => "\u{00BF}",
        "textmu" => "\u{00B5}",
        "textohm" => "\u{2126}",
        "textnumero" => "\u{2116}",
        "slash" => "/",
        "lbrack" => "[",
        "rbrack" => "]",
        "lq" => "\u{2018}",
        "rq" => "\u{2019}",
        "dh" => "\u{00F0}",
        "DH" => "\u{00D0}",
        "th" => "\u{00FE}",
        "TH" => "\u{00DE}",
        "ng" => "\u{014B}",
        "NG" => "\u{014A}",
        "i" => "\u{0131}",
        "j" => "\u{0237}",
        _ => return None,
    })
}

/// Apply an accent to a base character, preferring precomposed characters and
/// falling back to a Unicode combining mark.
fn apply_accent_string(accent: &crate::lexer_extended::AccentType, base: char) -> String {
    let composed = apply_accent(accent.clone(), base);
    if composed != base {
        return composed.to_string();
    }
    match accent_combining_mark(accent) {
        Some(mark) if base.is_alphanumeric() => format!("{}{}", base, mark),
        _ => base.to_string(),
    }
}

fn accent_combining_mark(accent: &crate::lexer_extended::AccentType) -> Option<char> {
    use crate::lexer_extended::AccentType::*;
    Some(match accent {
        Acute => '\u{0301}',
        Grave => '\u{0300}',
        Circumflex => '\u{0302}',
        Tilde => '\u{0303}',
        Diaeresis => '\u{0308}',
        Macron => '\u{0304}',
        Dot => '\u{0307}',
        Breve => '\u{0306}',
        Caron => '\u{030C}',
        DoubleAcute => '\u{030B}',
        Cedilla => '\u{0327}',
        Ogonek => '\u{0328}',
        Ring => '\u{030A}',
        Tie => '\u{0361}',
    })
}

/// The character an accent command produces with an empty argument (\~{} etc.)
fn standalone_accent_text(accent: &crate::lexer_extended::AccentType) -> String {
    use crate::lexer_extended::AccentType::*;
    match accent {
        Acute => "\u{00B4}",
        Grave => "`",
        Circumflex => "^",
        Tilde => "~",
        Diaeresis => "\u{00A8}",
        Macron => "\u{00AF}",
        _ => "",
    }
    .to_string()
}

/// Font declaration commands that style the rest of the enclosing group.
enum DeclStyle {
    Strong,
    Emph,
    SmallCaps,
    Mono,
    Plain,
}

/// Apply a font declaration to the text of a block (paragraphs, list
/// items, quotes); structural blocks are left as they are.
fn style_block(style: &DeclStyle, block: Block) -> Block {
    match block {
        Block::Paragraph(inlines) => Block::Paragraph(apply_decl_style(style, inlines)),
        Block::Quote(blocks) => Block::Quote(blocks.into_iter().map(|b| style_block(style, b)).collect()),
        Block::Composite(blocks) => Block::Composite(blocks.into_iter().map(|b| style_block(style, b)).collect()),
        Block::BulletList(items) => Block::BulletList(
            items
                .into_iter()
                .map(|item| item.into_iter().map(|b| style_block(style, b)).collect())
                .collect(),
        ),
        other => other,
    }
}

/// Wrap inlines in the inline type a font declaration stands for.
fn apply_decl_style(style: &DeclStyle, inlines: Vec<Inline>) -> Vec<Inline> {
    if inlines.is_empty() {
        return inlines;
    }
    match style {
        DeclStyle::Strong => vec![Inline::Strong(inlines)],
        DeclStyle::Emph => vec![Inline::Emph(inlines)],
        DeclStyle::SmallCaps => vec![Inline::SmallCaps(inlines)],
        DeclStyle::Mono => {
            // Monospace text is code when it is plain text
            if inlines.iter().all(|i| matches!(i, Inline::Text(_) | Inline::Space)) {
                let text: String = inlines
                    .iter()
                    .map(|i| match i {
                        Inline::Text(t) => t.as_str(),
                        _ => " ",
                    })
                    .collect();
                vec![Inline::Code(text.trim().to_string())]
            } else {
                inlines
            }
        }
        DeclStyle::Plain => inlines,
    }
}

fn declaration_style(cmd: &str) -> Option<DeclStyle> {
    Some(match cmd {
        "bf" | "bfseries" => DeclStyle::Strong,
        "it" | "itshape" | "em" | "sl" | "slshape" => DeclStyle::Emph,
        "sc" | "scshape" => DeclStyle::SmallCaps,
        "tt" | "ttfamily" => DeclStyle::Mono,
        "rm" | "sf" | "rmfamily" | "sffamily" | "normalfont"
        | "mdseries" | "upshape" | "normalsize" | "small" | "footnotesize" | "scriptsize"
        | "tiny" | "large" | "Large" | "LARGE" | "huge" | "Huge" => DeclStyle::Plain,
        _ => return None,
    })
}

/// Horizontal rule / styling commands that appear inside tabular bodies and
/// produce no cell content.
fn is_table_rule_command(cmd: &str) -> bool {
    matches!(
        cmd,
        "hline"
            | "toprule"
            | "midrule"
            | "bottomrule"
            | "cline"
            | "cmidrule"
            | "cmidrules"
            | "morecmidrules"
            | "addlinespace"
            | "specialrule"
            | "rowcolor"
            | "arrayrulecolor"
            | "noalign"
            | "endfirsthead"
            | "endhead"
            | "endfoot"
            | "endlastfoot"
    )
}

/// Drop leading spaces (the space after a declaration like `\bf ` is
/// skipped by TeX's tokenizer).
fn trim_leading_space(mut inlines: Vec<Inline>) -> Vec<Inline> {
    while matches!(inlines.first(), Some(Inline::Space)) {
        inlines.remove(0);
    }
    inlines
}

/// Strip leading/trailing whitespace-like inlines from a cell or fragment.
fn trim_inline_whitespace(mut inlines: Vec<Inline>) -> Vec<Inline> {
    while matches!(
        inlines.first(),
        Some(Inline::Space) | Some(Inline::SoftBreak) | Some(Inline::LineBreak)
    ) {
        inlines.remove(0);
    }
    while matches!(
        inlines.last(),
        Some(Inline::Space) | Some(Inline::SoftBreak) | Some(Inline::LineBreak)
    ) {
        inlines.pop();
    }
    inlines
}

/// Split a leading `[...]` group off a raw environment body, returning
/// (bracket content, remainder).
fn split_leading_bracket_group(body: &str) -> (Option<String>, String) {
    let trimmed = body.trim_start();
    if let Some(rest) = trimmed.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            let options = rest[..end].to_string();
            let remainder = rest[end + 1..]
                .strip_prefix('\n')
                .unwrap_or(&rest[end + 1..])
                .to_string();
            return (Some(options), remainder);
        }
    }
    (None, body.to_string())
}

/// Split a leading `{...}` group off a raw environment body, returning
/// (brace content, remainder).
fn split_leading_brace_group(body: &str) -> (Option<String>, String) {
    let trimmed = body.trim_start();
    if let Some(rest) = trimmed.strip_prefix('{') {
        if let Some(end) = rest.find('}') {
            let group = rest[..end].to_string();
            let remainder = rest[end + 1..]
                .strip_prefix('\n')
                .unwrap_or(&rest[end + 1..])
                .to_string();
            return (Some(group), remainder);
        }
    }
    (None, body.to_string())
}

/// Split display-math content into rows at top-level `\\\\` separators
/// (ignoring those nested in braces or inner environments like `cases`).
/// Returns (row text, separator text) pairs; the separator keeps optional
/// spacing like `\\\\[2pt]`.
fn split_math_rows(body: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = body.chars().collect();
    let mut rows = Vec::new();
    let mut current = String::new();
    let mut depth: i32 = 0;
    let mut env_depth: i32 = 0;
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '\\' && i + 1 < chars.len() {
            let next = chars[i + 1];
            if next == '\\' && depth == 0 && env_depth == 0 {
                let mut sep = String::from("\\\\");
                i += 2;
                if i < chars.len() && chars[i] == '*' {
                    i += 1;
                }
                // optional [len]
                let mut j = i;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if j < chars.len() && chars[j] == '[' {
                    if let Some(close) = chars[j..].iter().position(|&c| c == ']') {
                        let spacing: String = chars[j..j + close + 1].iter().collect();
                        sep.push_str(&spacing);
                        i = j + close + 1;
                    }
                }
                rows.push((std::mem::take(&mut current), sep));
                continue;
            }
            let rest: String = chars[i + 1..chars.len().min(i + 7)].iter().collect();
            if rest.starts_with("begin") {
                env_depth += 1;
            } else if rest.starts_with("end") {
                env_depth -= 1;
            }
            current.push(ch);
            current.push(next);
            i += 2;
            continue;
        }
        match ch {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        current.push(ch);
        i += 1;
    }
    rows.push((current, String::new()));
    rows
}

struct MathRow {
    text: String,
    labels: Vec<String>,
    nonumber: bool,
    tag: Option<String>,
}

/// Remove \\label, \\nonumber/\\notag and \\tag from a math row (recording
/// them), plus layout commands that Markdown math renderers reject.
fn strip_math_row_markup(row: &str) -> MathRow {
    let mut info = MathRow {
        text: String::new(),
        labels: Vec::new(),
        nonumber: false,
        tag: None,
    };
    let chars: Vec<char> = row.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1].is_ascii_alphabetic() {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_ascii_alphabetic() {
                j += 1;
            }
            let name: String = chars[i + 1..j].iter().collect();
            let mut k = j;
            if k < chars.len() && chars[k] == '*' && name == "tag" {
                k += 1;
            }
            let read_group = |start: usize| -> Option<(String, usize)> {
                let mut p = start;
                while p < chars.len() && chars[p].is_whitespace() {
                    p += 1;
                }
                if p < chars.len() && chars[p] == '{' {
                    let mut depth = 0;
                    let mut q = p;
                    while q < chars.len() {
                        match chars[q] {
                            '{' => depth += 1,
                            '}' => {
                                depth -= 1;
                                if depth == 0 {
                                    return Some((chars[p + 1..q].iter().collect(), q + 1));
                                }
                            }
                            _ => {}
                        }
                        q += 1;
                    }
                }
                None
            };
            match name.as_str() {
                "label" => {
                    if let Some((label, next)) = read_group(k) {
                        info.labels.push(label.trim().to_string());
                        i = next;
                        continue;
                    }
                }
                "nonumber" | "notag" => {
                    info.nonumber = true;
                    i = j;
                    continue;
                }
                "tag" => {
                    if let Some((tag, next)) = read_group(k) {
                        info.tag = Some(tag.trim().to_string());
                        i = next;
                        continue;
                    }
                }
                "displaybreak" | "allowdisplaybreaks" => {
                    i = j;
                    if i < chars.len() && chars[i] == '[' {
                        if let Some(close) = chars[i..].iter().position(|&c| c == ']') {
                            i += close + 1;
                        }
                    }
                    continue;
                }
                "intertext" | "shortintertext" => {
                    if let Some((text, next)) = read_group(k) {
                        info.text.push_str(&format!("\\text{{{}}}", text));
                        i = next;
                        continue;
                    }
                }
                _ => {}
            }
            info.text.extend(&chars[i..j]);
            i = j;
            continue;
        }
        info.text.push(chars[i]);
        i += 1;
    }
    info
}

/// eqnarray rows use three columns (`a &=& b`); aligned wants `a &= b`.
fn eqnarray_row_to_aligned(row: &str) -> String {
    let mut out = String::with_capacity(row.len());
    let mut depth = 0;
    let mut amps = 0;
    let mut prev_backslash = false;
    for ch in row.chars() {
        if ch == '&' && depth == 0 && !prev_backslash {
            amps += 1;
            if amps == 2 {
                prev_backslash = false;
                continue;
            }
        }
        match ch {
            '{' if !prev_backslash => depth += 1,
            '}' if !prev_backslash => depth -= 1,
            _ => {}
        }
        prev_backslash = ch == '\\' && !prev_backslash;
        out.push(ch);
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum RefStyle {
    /// cleveref: lowercase names in \cref, eq. (1)
    Cleveref,
    /// hyperref \autoref: capitalized names, plain numbers
    Hyperref,
}

/// Name for a referenced label's type, and whether its number is shown in
/// parentheses (equations under cleveref).
fn ref_type_name(
    info: &crate::state::LabelInfo,
    capital: bool,
    plural: bool,
    style: RefStyle,
) -> (String, bool) {
    let (singular, parens) = match &info.label_type {
        LabelType::Section => ("section", false),
        LabelType::Figure if style == RefStyle::Cleveref && !capital => ("fig.", false),
        LabelType::Figure => ("figure", false),
        LabelType::Table => ("table", false),
        LabelType::Equation if style == RefStyle::Cleveref => {
            (if capital { "eq." } else { "eq." }, true)
        }
        LabelType::Equation => ("equation", false),
        LabelType::Theorem => ("theorem", false),
        LabelType::Lemma => ("lemma", false),
        LabelType::Definition => ("definition", false),
        LabelType::Other(name) => {
            let lower = name.to_lowercase();
            let mut word = lower.clone();
            if plural {
                word = pluralize(&word);
            }
            return (if capital { capitalize_first(&word) } else { word }, false);
        }
    };
    let mut word = singular.to_string();
    if plural {
        word = match word.as_str() {
            "fig." => "figs.".to_string(),
            "eq." => "eqs.".to_string(),
            other => pluralize(other),
        };
    }
    if capital {
        word = match word.as_str() {
            "fig." => "Figure".to_string(),
            "figs." => "Figures".to_string(),
            other => capitalize_first(other),
        };
    }
    (word, parens)
}

fn pluralize(word: &str) -> String {
    if word.ends_with("ix") {
        format!("{}ices", &word[..word.len() - 2])
    } else if word.ends_with('s') || word.ends_with('x') {
        format!("{}es", word)
    } else if word.ends_with('y') && !word.ends_with("ey") {
        format!("{}ies", &word[..word.len() - 1])
    } else {
        format!("{}s", word)
    }
}

fn capitalize_first(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// `a`, `a and b`, `a, b and c`.
fn join_and(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        _ => format!("{} and {}", items[..items.len() - 1].join(", "), items[items.len() - 1]),
    }
}

/// Is this text a beamer overlay specification (`<2->`, `<+->`, `<1,3>`)?
fn is_overlay_spec(text: &str) -> bool {
    text.len() >= 3
        && text.starts_with('<')
        && text.ends_with('>')
        && text[1..text.len() - 1]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '+' | ',' | '|' | '.' | '@' | ' ' | '(' | ')'))
}

/// A list of inlines as one inline.
fn span_or_single(mut content: Vec<Inline>) -> Inline {
    if content.len() == 1 {
        content.pop().unwrap()
    } else {
        Inline::Span {
            attrs: HashMap::new(),
            content,
        }
    }
}

/// Does a text token look like part of a TeX number/dimension/glue value
/// (`0pt`, `=11`, `plus`, `1fil`, `-.5em`, `by`)?
fn is_tex_value_text(text: &str) -> bool {
    let text = text.trim_start_matches('=');
    if text.is_empty() {
        return true;
    }
    if matches!(
        text,
        "plus" | "minus" | "by" | "at" | "scaled" | "true" | "fil" | "fill" | "filll" | "to" | "spread"
    ) {
        return true;
    }
    let numeric_end = text
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '+' | '"' | '\'')))
        .unwrap_or(text.len());
    if numeric_end == 0 {
        return false;
    }
    let unit = &text[numeric_end..];
    matches!(
        unit,
        "" | "pt" | "em" | "ex" | "in" | "cm" | "mm" | "bp" | "pc" | "sp" | "dd" | "cc" | "mu"
            | "fil" | "fill" | "filll" | "px" | "truept" | "truein" | "truecm"
    )
}

/// Parse TeX parameter text (`#1#2`, `#1.#2\par`, `(#1)`) into
/// (prefix before #1, delimiter after each parameter, parameter count).
fn parse_tex_param_text(text: &str) -> (String, Vec<String>, usize) {
    let mut prefix = String::new();
    let mut delimiters: Vec<String> = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '#' && chars.peek().is_some_and(|c| c.is_ascii_digit()) {
            chars.next();
            delimiters.push(String::new());
        } else if let Some(last) = delimiters.last_mut() {
            last.push(ch);
        } else {
            prefix.push(ch);
        }
    }
    let count = delimiters.len().min(9);
    delimiters.truncate(count);
    (prefix, delimiters, count)
}

/// Is the whole string a single balanced `{...}` group?
fn balanced_outer_group(text: &str) -> bool {
    let mut depth = 0i32;
    for (i, ch) in text.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 && i != text.len() - 1 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

/// Block-level commands with no output of their own, safe to process when
/// they appear inside a paragraph (definitions, metadata, preamble setup).
fn is_inline_safe_block_command(cmd: &str) -> bool {
    matches!(
        cmd,
        "newcommand"
            | "renewcommand"
            | "providecommand"
            | "def"
            | "gdef"
            | "edef"
            | "xdef"
            | "DeclareRobustCommand"
            | "newrobustcmd"
            | "renewrobustcmd"
            | "providerobustcmd"
            | "DeclareMathOperator"
            | "NewDocumentCommand"
            | "RenewDocumentCommand"
            | "ProvideDocumentCommand"
            | "DeclareDocumentCommand"
            | "newenvironment"
            | "renewenvironment"
            | "newtheorem"
            | "bibliography"
            | "addbibresource"
            | "bibliographystyle"
            | "title"
            | "author"
            | "date"
            | "graphicspath"
            | "usepackage"
            | "RequirePackage"
            | "documentclass"
            | "documentstyle"
    )
}

/// Put a custom `\item[label]` marker in front of the item's first paragraph.
fn prepend_item_label(blocks: &mut Vec<Block>, label: Vec<Inline>) {
    match blocks.first_mut() {
        Some(Block::Paragraph(inlines)) => {
            let mut new_inlines = label;
            new_inlines.push(Inline::Space);
            new_inlines.append(inlines);
            *inlines = new_inlines;
        }
        _ => blocks.insert(0, Block::Paragraph(label)),
    }
}

/// Extract `language=...` from lstlisting options.
fn extract_listing_language(options: &str) -> Option<String> {
    options.split(',').find_map(|opt| {
        let mut parts = opt.splitn(2, '=');
        let key = parts.next()?.trim();
        if key == "language" {
            parts.next().map(|value| value.trim().to_lowercase())
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    #[test]
    fn test_parse_paragraph() {
        let mut lexer = Lexer::new("Hello world");
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let doc = parser.parse().unwrap();
        assert_eq!(doc.blocks.len(), 1);
    }

    #[test]
    fn test_parse_section() {
        let mut lexer = Lexer::new("\\section{Introduction}");
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let doc = parser.parse().unwrap();
        assert!(matches!(doc.blocks[0], Block::Section { .. }));
    }
}
