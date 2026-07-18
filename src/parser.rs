use crate::citation::{CitationManager, CitationMode, parse_citation_args};
use crate::commands::{apply_accent, special_char_to_string, CommandRegistry};
use crate::error::Result;
use crate::include_system::IncludeSystem;
use crate::lexer::Lexer;
use crate::lexer::Token;
use crate::lexer_extended::{get_accent_type, get_special_char};
use crate::macro_processor::MacroProcessor;
use crate::state::{LabelType, MacroDef, ParserState};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Upper bound on macro expansions per document, guarding against
/// self-referential macro definitions blowing up the token stream.
const MAX_MACRO_EXPANSIONS: usize = 10_000;

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
        title: Option<String>,
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
    tokens: Vec<Token>,
    position: usize,
    metadata: HashMap<String, String>,
    state: ParserState,
    citation_manager: CitationManager,
    macro_processor: MacroProcessor,
    include_system: IncludeSystem,
    command_registry: CommandRegistry,
    base_path: Option<std::path::PathBuf>,
    macro_expansions: usize,
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
        
        Self {
            tokens,
            position: 0,
            metadata: HashMap::new(),
            state: ParserState::new(),
            citation_manager: CitationManager::new(CitationStyle::Numeric),
            macro_processor: MacroProcessor::new(),
            include_system: IncludeSystem::new(),
            command_registry: CommandRegistry::new(),
            base_path: None,
            macro_expansions: 0,
            footnote_inlines: HashMap::new(),
        }
    }
    
    pub fn with_base_path(mut self, path: std::path::PathBuf) -> Self {
        self.base_path = Some(path);
        self
    }

    pub fn parse(&mut self) -> Result<Document> {
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
            Block::TheoremLike { content, .. } => self.resolve_references_in_blocks(content),
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
                    if self.state.get_label(label).is_some() {
                        *inline = self.render_reference(&kind.clone(), &label.clone());
                    } else {
                        let warning =
                            format!("unresolved reference `\\{}{{{}}}`", kind, label);
                        self.state.warn(warning);
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
                    if let Ok(content) = fs::read_to_string(path) {
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
                | "par"
                | "noindent"
                | "hline"
                | "hrule"
                | "include"
                | "input"
                | "usepackage"
                | "newcommand"
                | "renewcommand"
                | "providecommand"
                | "def"
                | "DeclareRobustCommand"
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
        let token = self.current_token();

        match token {
            Token::BeginEnvironment(env) => self.parse_environment(env.clone()),
            Token::EndEnvironment(env) if self.state.environments.contains_key(env) => {
                // Closing a user-defined environment: splice its end-definition
                let end_def = self.state.environments[env].end_def.clone();
                self.advance();
                self.splice_fragment_tokens(&end_def)?;
                Ok(None)
            }
            Token::Command(cmd) => {
                if self.is_block_command(cmd) {
                    self.parse_command_block(cmd.clone())
                } else {
                    self.parse_paragraph()
                }
            }
            Token::Section(level, title) => {
                let level = *level;
                let title_text = title.clone();
                self.advance();
                self.state.increment_section(level as usize);

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
                    self.state.add_label(
                        section_label.clone(),
                        LabelType::Section,
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
                let content = self.expand_macros_in_math(content);
                self.advance();
                Ok(Some(Block::DisplayMath(content)))
            }
            Token::VerbatimEnv { name, content } => {
                let name = name.clone();
                let content = content.clone();
                self.advance();
                Ok(self.verbatim_env_to_block(&name, content))
            }
            Token::Text(_)
            | Token::InlineMath(_)
            | Token::LeftBrace
            | Token::Ref { .. }
            | Token::Cite { .. }
            | Token::Verb(_)
            | Token::Tilde
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
                // Document environment - just skip the begin/end tags, parse contents normally
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
            "table" | "table*" | "sidewaystable" | "sidewaystable*" => {
                self.parse_table_environment(&env_name)
            }
            "tabular" | "tabular*" | "tabularx" | "longtable" | "array" | "tabu"
            | "supertabular" => self.parse_tabular(&env_name),
            "figure" | "figure*" | "sidewaysfigure" | "wrapfigure" => {
                self.parse_figure_environment(&env_name)
            }
            "center" | "flushleft" | "flushright" | "sloppypar" | "samepage" | "titlepage"
            | "singlespace" | "landscape" | "footnotesize" | "small" | "large" => {
                self.parse_transparent_environment(&env_name, 0, false)
            }
            "minipage" => self.parse_transparent_environment(&env_name, 1, true),
            "multicols" | "spacing" => self.parse_transparent_environment(&env_name, 1, false),
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

        let mut body = def.begin_def.clone();
        for (i, arg) in args.iter().enumerate() {
            body = body.replace(&format!("#{}", i + 1), arg);
        }

        self.splice_fragment_tokens(&body)?;
        Ok(None)
    }

    /// Lex a LaTeX fragment and splice its tokens at the current position.
    fn splice_fragment_tokens(&mut self, fragment: &str) -> Result<()> {
        if fragment.trim().is_empty() {
            return Ok(());
        }
        let mut lexer = Lexer::new(fragment);
        let mut tokens = lexer.tokenize()?;
        if matches!(tokens.last(), Some(Token::Eof)) {
            tokens.pop();
        }
        self.tokens.splice(self.position..self.position, tokens);
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
        let mut items = Vec::new();

        // Optional list options: \begin{itemize}[noitemsep]
        self.skip_whitespace_and_comments();
        if matches!(self.current_token(), Token::LeftBracket) {
            let _ = self.parse_bracketed_text()?;
        }

        while !self.is_at_end() {
            self.skip_whitespace_and_comments();

            if let Token::EndEnvironment(env) = self.current_token() {
                if env == "itemize" {
                    self.advance();
                    break;
                }
            }

            if matches!(self.current_token(), Token::Item) {
                self.advance();
                self.skip_item_marker()?;
                let item_blocks = self.parse_item_content("itemize")?;
                items.push(item_blocks);
            } else {
                self.advance();
            }
        }

        Ok(Some(Block::BulletList(items)))
    }

    /// Skip a custom item marker: \item[--]
    fn skip_item_marker(&mut self) -> Result<()> {
        let mut lookahead = self.position;
        while matches!(
            self.tokens.get(lookahead),
            Some(Token::Whitespace(_)) | Some(Token::Newline)
        ) {
            lookahead += 1;
        }
        if matches!(self.tokens.get(lookahead), Some(Token::LeftBracket)) {
            self.position = lookahead;
            let _ = self.parse_bracketed_text()?;
        }
        Ok(())
    }

    fn parse_enumerate(&mut self) -> Result<Option<Block>> {
        let mut items = Vec::new();

        // Optional list options: \begin{enumerate}[label=...]
        self.skip_whitespace_and_comments();
        if matches!(self.current_token(), Token::LeftBracket) {
            let _ = self.parse_bracketed_text()?;
        }

        while !self.is_at_end() {
            self.skip_whitespace_and_comments();

            if let Token::EndEnvironment(env) = self.current_token() {
                if env == "enumerate" {
                    self.advance();
                    break;
                }
            }

            if matches!(self.current_token(), Token::Item) {
                self.advance();
                self.skip_item_marker()?;
                let item_blocks = self.parse_item_content("enumerate")?;
                items.push(item_blocks);
            } else {
                self.advance();
            }
        }

        Ok(Some(Block::OrderedList { start: 1, items }))
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
                
                // Parse optional term in brackets
                let term = if matches!(self.current_token(), Token::LeftBracket) {
                    self.advance();
                    let term_inlines = self.parse_inlines_until(Token::RightBracket)?;
                    if matches!(self.current_token(), Token::RightBracket) {
                        self.advance();
                    }
                    term_inlines
                } else {
                    vec![Inline::Text(String::from("Item"))]
                };

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
        // alignat takes a {n} argument specifying the number of columns
        if env_name.starts_with("alignat") {
            self.skip_whitespace_and_comments();
            let _ = self.parse_braced_text()?;
        }

        let mut content = String::new();
        let mut has_label = false;
        let mut label = String::new();

        while !self.is_at_end() {
            if let Token::EndEnvironment(env) = self.current_token() {
                if env == env_name {
                    self.advance();
                    break;
                }
            }

            // Check for label
            if let Token::Label(lbl) = self.current_token() {
                label = lbl.clone();
                has_label = true;
                self.advance();

                // Add label to state for equation numbering
                let is_numbered = !env_name.ends_with('*');
                if is_numbered {
                    use crate::state::LabelType;
                    self.state.add_label(label.clone(), LabelType::Equation, None);
                }
                continue;
            }

            // Numbering suppression has no meaning in Markdown math
            if let Token::Command(cmd) = self.current_token() {
                if cmd == "nonumber" || cmd == "notag" {
                    self.advance();
                    continue;
                }
            }

            content.push_str(&self.token_to_string(self.current_token()));
            self.advance();
        }

        let mut content = self.expand_macros_in_math(content.trim());

        // For numbered environments with a label, add the equation number
        let is_numbered = !env_name.ends_with('*');
        if is_numbered && has_label {
            if let Some(label_info) = self.state.get_label(&label) {
                if !label_info.number.is_empty() {
                    content = format!("{}\\tag{{{}}}", content, label_info.number);
                }
            }
        }

        // Multi-line alignment environments keep their structure by being
        // wrapped in aligned/gathered, which MathJax/KaTeX render natively.
        let base = env_name.trim_end_matches('*');
        let content = match base {
            "align" | "alignat" | "flalign" | "eqnarray" | "split" | "multline" => {
                format!("\\begin{{aligned}}\n{}\n\\end{{aligned}}", content)
            }
            "gather" => format!("\\begin{{gathered}}\n{}\n\\end{{gathered}}", content),
            _ => content,
        };

        Ok(Some(Block::DisplayMath(content)))
    }

    fn parse_table_environment(&mut self, env_name: &str) -> Result<Option<Block>> {
        let mut caption = None;
        let mut tabular_block = None;
        let mut label = None;

        // Skip optional placement specifier [htbp]
        self.skip_whitespace_and_comments();
        if matches!(self.current_token(), Token::LeftBracket) {
            let _ = self.parse_bracketed_text()?;
        }

        while !self.is_at_end() {
            self.skip_whitespace_and_comments();

            if let Token::EndEnvironment(env) = self.current_token() {
                if env == env_name {
                    self.advance();
                    break;
                }
            }

            if let Token::Command(cmd) = self.current_token() {
                if cmd == "caption" {
                    self.advance();
                    self.consume_text_star();
                    if matches!(self.current_token(), Token::LeftBracket) {
                        let _ = self.parse_bracketed_text()?;
                    }
                    let tokens = self.collect_braced_tokens();
                    let (inlines, caption_label) = self.parse_inline_fragment_tokens(tokens)?;
                    caption = Some(inlines);
                    if caption_label.is_some() {
                        label = caption_label;
                    }
                    continue;
                }
            }

            if let Token::BeginEnvironment(env) = self.current_token() {
                if matches!(
                    env.as_str(),
                    "tabular" | "tabular*" | "tabularx" | "longtable" | "array" | "tabu"
                        | "supertabular"
                ) {
                    tabular_block = self.parse_environment(env.clone())?;
                    continue;
                }
            }

            if let Token::Label(l) = self.current_token() {
                label = Some(l.clone());
                self.advance();
                continue;
            }

            self.advance();
        }

        if let Some(Block::Table { alignments, headers, rows, .. }) = tabular_block {
            if let Some(table_label) = &label {
                let next_number = self.state.table_counter + 1;
                let title = caption
                    .as_ref()
                    .map(|inlines| self.inlines_to_plain_text(inlines))
                    .filter(|text| !text.is_empty());
                self.state.add_label_with_number(
                    table_label.clone(),
                    LabelType::Table,
                    next_number.to_string(),
                    title,
                );
                self.state.table_counter = next_number;
            }
            Ok(Some(Block::Table {
                caption,
                alignments,
                headers,
                rows,
            }))
        } else {
            Ok(None)
        }
    }

    fn parse_tabular(&mut self, env_name: &str) -> Result<Option<Block>> {
        // Optional position argument [t]/[b]
        self.skip_whitespace_and_comments();
        if matches!(self.current_token(), Token::LeftBracket) {
            let _ = self.parse_bracketed_text()?;
        }

        // tabular* and tabularx take a width argument before the column spec
        if matches!(env_name, "tabular*" | "tabularx" | "tabu") {
            self.skip_whitespace_and_comments();
            let _ = self.parse_braced_text()?;
        }

        // Parse column specification
        self.skip_whitespace_and_comments();
        let alignments = if matches!(self.current_token(), Token::LeftBrace) {
            let spec = self.parse_braced_text()?;
            self.parse_column_spec(&spec)
        } else {
            vec![Alignment::AlignDefault]
        };

        let mut rows: Vec<Vec<Vec<Block>>> = Vec::new();
        let mut current_row: Vec<Vec<Block>> = Vec::new();
        let mut cell_inlines: Vec<Inline> = Vec::new();
        // Extra empty cells owed to \multicolumn spans
        let mut pending_span_cells: usize = 0;

        fn finish_cell(
            cell_inlines: &mut Vec<Inline>,
            current_row: &mut Vec<Vec<Block>>,
            pending_span_cells: &mut usize,
        ) {
            let inlines = std::mem::take(cell_inlines);
            let trimmed = trim_inline_whitespace(inlines);
            if trimmed.is_empty() {
                current_row.push(Vec::new());
            } else {
                current_row.push(vec![Block::Paragraph(trimmed)]);
            }
            for _ in 0..*pending_span_cells {
                current_row.push(Vec::new());
            }
            *pending_span_cells = 0;
        }

        while !self.is_at_end() {
            match self.current_token() {
                Token::EndEnvironment(env) if env == env_name => {
                    self.advance();
                    break;
                }
                Token::Ampersand => {
                    self.advance();
                    finish_cell(&mut cell_inlines, &mut current_row, &mut pending_span_cells);
                }
                Token::Backslash => {
                    self.advance();
                    // Optional row spacing: \\[3pt]
                    if matches!(self.current_token(), Token::LeftBracket) {
                        let _ = self.parse_bracketed_text()?;
                    }
                    finish_cell(&mut cell_inlines, &mut current_row, &mut pending_span_cells);
                    rows.push(std::mem::take(&mut current_row));
                }
                Token::Command(cmd) if is_table_rule_command(cmd) => {
                    let cmd = cmd.clone();
                    self.advance();
                    self.consume_table_rule_args(&cmd)?;
                }
                Token::Command(cmd) if cmd == "multicolumn" => {
                    self.advance();
                    let span = self
                        .parse_braced_text()?
                        .trim()
                        .parse::<usize>()
                        .unwrap_or(1);
                    let _alignment = self.parse_braced_text()?;
                    let tokens = self.collect_braced_tokens();
                    let (inlines, _) = self.parse_inline_fragment_tokens(tokens)?;
                    cell_inlines.extend(inlines);
                    pending_span_cells = pending_span_cells.max(span.saturating_sub(1));
                }
                Token::Command(cmd) if cmd == "multirow" => {
                    self.advance();
                    let _rows = self.parse_braced_text()?;
                    if matches!(self.current_token(), Token::LeftBracket) {
                        let _ = self.parse_bracketed_text()?;
                    }
                    let _width = self.parse_braced_text()?;
                    let tokens = self.collect_braced_tokens();
                    let (inlines, _) = self.parse_inline_fragment_tokens(tokens)?;
                    cell_inlines.extend(inlines);
                }
                Token::Label(_) | Token::Comment(_) => {
                    self.advance();
                }
                _ => {
                    if let Some(inline) = self.parse_inline()? {
                        cell_inlines.push(inline);
                    }
                }
            }
        }

        // Final row without trailing \\
        if !cell_inlines.is_empty()
            || !current_row.is_empty()
            || pending_span_cells > 0
        {
            finish_cell(&mut cell_inlines, &mut current_row, &mut pending_span_cells);
            rows.push(current_row);
        }

        // Drop rows that are entirely empty (e.g. produced by a trailing \\)
        rows.retain(|row| row.iter().any(|cell| !cell.is_empty()));

        let headers = if !rows.is_empty() {
            rows.remove(0)
        } else {
            Vec::new()
        };

        Ok(Some(Block::Table {
            caption: None,
            alignments,
            headers,
            rows,
        }))
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
                        alignments.extend(inner_alignments.iter().cloned());
                    }
                }
                _ => {}
            }
        }

        alignments
    }

    fn parse_figure_environment(&mut self, env_name: &str) -> Result<Option<Block>> {
        let mut caption = None;
        let mut paths: Vec<String> = Vec::new();
        let mut label = None;

        // Skip optional placement specifier [htbp] (or {width} for wrapfigure)
        self.skip_whitespace_and_comments();
        if matches!(self.current_token(), Token::LeftBracket) {
            let _ = self.parse_bracketed_text()?;
        }

        while !self.is_at_end() {
            self.skip_whitespace_and_comments();

            if let Token::EndEnvironment(env) = self.current_token() {
                if env == env_name {
                    self.advance();
                    break;
                }
            }

            if let Token::Command(cmd) = self.current_token() {
                match cmd.as_str() {
                    "includegraphics" => {
                        self.advance();
                        self.consume_text_star();
                        // Skip optional arguments
                        if matches!(self.current_token(), Token::LeftBracket) {
                            let _ = self.parse_bracketed_text()?;
                        }
                        // Get file path
                        let path = self.parse_braced_text()?;
                        if !path.is_empty() {
                            paths.push(path);
                        }
                    }
                    "caption" => {
                        self.advance();
                        self.consume_text_star();
                        if matches!(self.current_token(), Token::LeftBracket) {
                            let _ = self.parse_bracketed_text()?;
                        }
                        let tokens = self.collect_braced_tokens();
                        let (inlines, caption_label) =
                            self.parse_inline_fragment_tokens(tokens)?;
                        caption = Some(inlines);
                        if caption_label.is_some() {
                            label = caption_label;
                        }
                    }
                    _ => {
                        self.advance();
                    }
                }
                continue;
            }

            if let Token::Label(l) = self.current_token() {
                label = Some(l.clone());
                self.advance();
                continue;
            }

            self.advance();
        }

        if let Some(figure_label) = &label {
            let next_number = self.state.figure_counter + 1;
            let title = caption
                .as_ref()
                .map(|inlines| self.inlines_to_plain_text(inlines))
                .filter(|text| !text.is_empty());
            self.state.add_label_with_number(
                figure_label.clone(),
                LabelType::Figure,
                next_number.to_string(),
                title,
            );
            self.state.figure_counter = next_number;
        }

        match paths.len() {
            0 => {
                // Figure without graphics (e.g. only a tikzpicture was skipped):
                // still emit the caption text if we have one.
                if let Some(cap) = caption {
                    Ok(Some(Block::Paragraph(cap)))
                } else {
                    Ok(None)
                }
            }
            1 => Ok(Some(Block::Figure {
                caption,
                path: paths.into_iter().next().unwrap(),
                label,
            })),
            _ => {
                // Multiple images (subfigures): one Figure per image,
                // caption and label attached to the first.
                let mut blocks = Vec::new();
                let mut caption = caption;
                let mut label = label;
                for path in paths {
                    blocks.push(Block::Figure {
                        caption: caption.take(),
                        path,
                        label: label.take(),
                    });
                }
                Ok(Some(Block::Composite(blocks)))
            }
        }
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
        let title = if matches!(self.current_token(), Token::LeftBracket) {
            self.advance();
            let title_text = self.read_until_token(Token::RightBracket);
            if matches!(self.current_token(), Token::RightBracket) {
                self.advance();
            }
            Some(title_text)
        } else {
            None
        };

        let mut content = Vec::new();
        let mut label = None;

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
                title.clone(),
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
                | Token::Section(_, _)
                | Token::DisplayMath(_)
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
                _ => {
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
            if let Some(inline) = self.parse_inline()? {
                inlines.push(inline);
            }
        }

        Ok(inlines)
    }

    fn parse_inline(&mut self) -> Result<Option<Inline>> {
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
                let content = self.expand_macros_in_math(content);
                self.advance();
                Ok(Some(Inline::InlineMath(content)))
            }
            Token::Command(cmd) => self.parse_inline_command(cmd.clone()),
            Token::LeftBrace => self.parse_braced_group(),
            Token::Verb(code) => {
                let code = code.clone();
                self.advance();
                Ok(Some(Inline::Code(code)))
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

        // Check for citation commands
        if self.is_citation_command(&cmd) {
            return self.parse_citation_command(&cmd);
        }

        // Check for reference commands
        if self.is_reference_command(&cmd) {
            return self.parse_reference_command(&cmd);
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
                self.state
                    .warn(format!("unknown command `\\{}` kept as plain text", cmd));
                Ok(Some(Inline::Text(format!("\\{}", cmd))))
            }
        }
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
            "ref" | "eqref" | "autoref" | "nameref" | "pageref" | "cref" | "Cref" | "vref"
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
        let label = self.parse_braced_text()?;
        Ok(Some(self.render_reference(cmd, &label)))
    }

    /// Render a \ref-family command against the label table.
    /// Unresolved labels stay as `Inline::Ref` so a later pass (after the
    /// whole document is parsed) can resolve forward references.
    fn render_reference(&self, kind: &str, label: &str) -> Inline {
        let Some(label_info) = self.state.get_label(label) else {
            return Inline::Ref {
                kind: kind.to_string(),
                label: label.to_string(),
            };
        };

        let type_prefix = || -> &str {
            match &label_info.label_type {
                LabelType::Section => "Section",
                LabelType::Figure => "Figure",
                LabelType::Table => "Table",
                LabelType::Equation => "Equation",
                LabelType::Theorem => "Theorem",
                LabelType::Lemma => "Lemma",
                LabelType::Definition => "Definition",
                LabelType::Other(name) => name.as_str(),
            }
        };

        let text = match kind {
            "nameref" => label_info
                .title
                .clone()
                .unwrap_or_else(|| label.to_string()),
            "eqref" => format!("({})", label_info.number),
            "autoref" | "Cref" | "cref" | "vref" => {
                let prefix = type_prefix();
                let prefix = if kind == "cref" {
                    prefix.to_lowercase()
                } else {
                    prefix.to_string()
                };
                if label_info.number.is_empty() {
                    prefix
                } else {
                    format!("{} {}", prefix, label_info.number)
                }
            }
            _ => label_info.number.clone(),
        };
        Inline::Text(text)
    }
    
    fn parse_macro_expansion(&mut self, cmd: &str) -> Result<Option<Inline>> {
        let args = self.parse_macro_arguments(cmd)?;

        if self.macro_expansions >= MAX_MACRO_EXPANSIONS {
            return Ok(Some(Inline::Text(format!("\\{}", cmd))));
        }

        if let Some(expanded) = self.macro_processor.expand_macro(&self.state, cmd, &args) {
            // Re-lex the expansion and splice it into the token stream so the
            // body is fully parsed (formatting, math, nested macros, ...).
            self.macro_expansions += 1;
            let mut lexer = Lexer::new(&expanded);
            let mut tokens = lexer.tokenize()?;
            if matches!(tokens.last(), Some(Token::Eof)) {
                tokens.pop();
            }
            self.tokens.splice(self.position..self.position, tokens);
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
            let mut lookahead = self.position;
            while matches!(self.tokens.get(lookahead), Some(Token::Whitespace(_))) {
                lookahead += 1;
            }
            match self.tokens.get(lookahead) {
                Some(Token::Command(cmd)) => match declaration_style(cmd) {
                    Some(style) => {
                        self.position = lookahead + 1;
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
            inlines = match style {
                DeclStyle::Strong => vec![Inline::Strong(inlines)],
                DeclStyle::Emph => vec![Inline::Emph(inlines)],
                DeclStyle::SmallCaps => vec![Inline::SmallCaps(inlines)],
                DeclStyle::Plain => inlines,
            };
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
        // Both \newcommand{\name} and the unbraced \newcommand\name are valid
        let name_raw = if matches!(self.current_token(), Token::LeftBrace) {
            self.parse_braced_text()?
        } else if let Token::Command(name) = self.current_token() {
            let name = name.clone();
            self.advance();
            name
        } else {
            String::new()
        };
        if name_raw.is_empty() {
            return Ok(());
        }

        let mut num_params = if matches!(self.current_token(), Token::LeftBracket) {
            self.parse_bracketed_text()?.trim().parse::<usize>().unwrap_or(0)
        } else {
            0
        };

        // TeX-style parameter text: \def\pair#1#2{...}
        if let Token::Text(text) = self.current_token() {
            if text.starts_with('#') {
                num_params = text.matches('#').count();
                self.advance();
            }
        }

        let optional_param = if matches!(self.current_token(), Token::LeftBracket) {
            Some(self.parse_bracketed_text()?)
        } else {
            None
        };

        let body = self.parse_braced_text()?;
        let name = name_raw.trim().trim_start_matches('\\');

        if !name.is_empty() {
            let macro_def = MacroDef {
                num_params,
                body,
                optional_param,
            };
            match mode {
                MacroDefinitionMode::New | MacroDefinitionMode::Renew => {
                    self.state.macros.insert(name.to_string(), macro_def);
                }
                MacroDefinitionMode::Provide => {
                    self.state.macros.entry(name.to_string()).or_insert(macro_def);
                }
            }
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

        let mut args = Vec::new();
        let required_params = if optional_param.is_some() {
            if matches!(self.current_token(), Token::LeftBracket) {
                args.push(self.parse_bracketed_text()?);
            } else if let Some(default) = optional_param {
                args.push(default);
            }
            num_params.saturating_sub(1)
        } else {
            num_params
        };

        for _ in 0..required_params {
            self.skip_whitespace_and_comments();
            args.push(self.parse_braced_text()?);
        }

        Ok(args)
    }

    /// Expand user-defined macros inside math content, so constructs like
    /// \newcommand{\R}{\mathbb{R}} render in Markdown math (KaTeX/MathJax
    /// don't know document macros).
    fn expand_macros_in_math(&self, content: &str) -> String {
        let mut chars: Vec<char> = content.chars().collect();
        let mut i = 0;
        let mut expansions = 0usize;

        while i < chars.len() {
            if chars[i] == '\\'
                && i + 1 < chars.len()
                && chars[i + 1].is_alphabetic()
                && expansions < 200
            {
                let start = i + 1;
                let mut end = start;
                while end < chars.len() && chars[end].is_alphabetic() {
                    end += 1;
                }
                let name: String = chars[start..end].iter().collect();

                if let Some(def) = self.state.macros.get(&name) {
                    let def = def.clone();
                    let mut args: Vec<String> = Vec::new();
                    let mut j = end;

                    let required = if let Some(default) = &def.optional_param {
                        if j < chars.len() && chars[j] == '[' {
                            let mut opt = String::new();
                            let mut k = j + 1;
                            while k < chars.len() && chars[k] != ']' {
                                opt.push(chars[k]);
                                k += 1;
                            }
                            args.push(opt);
                            j = (k + 1).min(chars.len());
                        } else {
                            args.push(default.clone());
                        }
                        def.num_params.saturating_sub(1)
                    } else {
                        def.num_params
                    };

                    for _ in 0..required {
                        while j < chars.len() && chars[j].is_whitespace() {
                            j += 1;
                        }
                        if j < chars.len() && chars[j] == '{' {
                            let mut depth = 1;
                            let mut k = j + 1;
                            let mut arg = String::new();
                            while k < chars.len() {
                                match chars[k] {
                                    '{' => depth += 1,
                                    '}' => {
                                        depth -= 1;
                                        if depth == 0 {
                                            break;
                                        }
                                    }
                                    _ => {}
                                }
                                arg.push(chars[k]);
                                k += 1;
                            }
                            args.push(arg);
                            j = (k + 1).min(chars.len());
                        } else if j < chars.len() {
                            args.push(chars[j].to_string());
                            j += 1;
                        }
                    }

                    if let Some(expansion) = self.state.expand_macro(&name, &args) {
                        expansions += 1;
                        let replacement: Vec<char> = expansion.chars().collect();
                        chars.splice(i..j, replacement);
                        continue; // rescan from the same position
                    }
                }
            }
            i += 1;
        }

        chars.into_iter().collect()
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

        let saved_tokens = std::mem::replace(&mut self.tokens, tokens);
        let saved_position = std::mem::replace(&mut self.position, 0);

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
        self.position = saved_position;
        result?;

        Ok((inlines, label))
    }

    /// Parse a LaTeX fragment and render it as a Markdown string
    /// (used for footnote texts and metadata values).
    fn render_fragment_markdown(&mut self, content: &str) -> Result<String> {
        let (inlines, _) = self.parse_inline_fragment(content)?;
        Ok(crate::converter::MarkdownConverter::render_inlines_fragment(&inlines))
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

        child.metadata = self.metadata.clone();
        child.state = self.state.clone();
        child.citation_manager = self.citation_manager.clone();
        child.macro_processor = self.macro_processor.clone();
        child.include_system = self.include_system.clone();
        child.command_registry = self.command_registry.clone();
        child.footnote_inlines = std::mem::take(&mut self.footnote_inlines);

        let include_base = self
            .include_system
            .get_resolved_path(filename, self.base_path.as_deref())
            .and_then(|path| path.parent().map(|p| p.to_path_buf()))
            .or_else(|| self.base_path.clone());
        if let Some(base_path) = include_base {
            child = child.with_base_path(base_path);
        }

        let blocks = child.parse_blocks()?;

        self.metadata = child.metadata;
        self.state = child.state;
        self.citation_manager = child.citation_manager;
        self.macro_processor = child.macro_processor;
        self.include_system = child.include_system;
        self.command_registry = child.command_registry;
        self.footnote_inlines = child.footnote_inlines;

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
            Token::VerbatimEnv { name, content } => {
                format!("\\begin{{{}}}{}\\end{{{}}}", name, content, name)
            }
            Token::Verb(v) => format!("\\verb|{}|", v),
            Token::Item => String::from("\\item "),
            Token::Label(l) => format!("\\label{{{}}}", l),
            Token::Ref { kind, label } => format!("\\{}{{{}}}", kind, label),
            Token::Cite { kind, keys } => format!("\\{}{{{}}}", kind, keys),
            Token::Section(level, title) => {
                let name = match level {
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
            "documentclass" => {
                self.advance();
                // Skip optional argument
                if matches!(self.current_token(), Token::LeftBracket) {
                    self.advance();
                    while !self.is_at_end() && !matches!(self.current_token(), Token::RightBracket) {
                        self.advance();
                    }
                    if matches!(self.current_token(), Token::RightBracket) {
                        self.advance();
                    }
                }
                // Skip required argument
                let _ = self.parse_braced_text()?;
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
            "usepackage" => {
                self.advance();
                let options = if matches!(self.current_token(), Token::LeftBracket) {
                    self.parse_bracketed_text()?
                } else {
                    String::new()
                };

                let package = self.parse_braced_text()?;
                if !package.is_empty() {
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
                    let _ = self.include_system.load_package(&mut self.state, &package, &[]);
                }
                Ok(None)
            }
            
            // Macro definitions
            "newcommand" | "renewcommand" | "providecommand" | "def" | "DeclareRobustCommand" => {
                self.advance();
                self.consume_text_star();
                let mode = match cmd.as_str() {
                    "renewcommand" | "def" => MacroDefinitionMode::Renew,
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
                let value = rendered
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .replace(" ,", ",")
                    .replace(",,", ",");
                let value = value
                    .trim()
                    .trim_start_matches(',')
                    .trim()
                    .to_string();
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
        &self.tokens[self.position]
    }

    fn advance(&mut self) {
        if !self.is_at_end() {
            self.position += 1;
        }
    }

    fn is_at_end(&self) -> bool {
        self.position >= self.tokens.len() || matches!(self.current_token(), Token::Eof)
    }
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
        | "subjclass" | "keywords" | "PARstart" | "captionsetup" | "floatplacement" => {
            Some((1, true))
        }

        // Two braced arguments
        "setlength" | "addtolength" | "setcounter" | "addtocounter" | "numberwithin"
        | "markboth" | "fontsize" | "DeclareMathOperator" | "settowidth"
        | "addcontentsline" | "PassOptionsToPackage" | "AtBeginDocument"
        | "RequirePackage" => Some((2, false)),

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
    Plain,
}

fn declaration_style(cmd: &str) -> Option<DeclStyle> {
    Some(match cmd {
        "bf" | "bfseries" => DeclStyle::Strong,
        "it" | "itshape" | "em" | "sl" | "slshape" => DeclStyle::Emph,
        "sc" | "scshape" => DeclStyle::SmallCaps,
        "rm" | "sf" | "tt" | "rmfamily" | "sffamily" | "ttfamily" | "normalfont"
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
