use crate::citation::{CitationManager, CitationMode, parse_citation_args};
use crate::commands::CommandRegistry;
use crate::error::Result;
use crate::include_system::IncludeSystem;
use crate::lexer::Lexer;
use crate::lexer::Token;
use crate::macro_processor::MacroProcessor;
use crate::state::{LabelType, MacroDef, ParserState};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

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
    Cite { citations: Vec<String>, content: Vec<Inline> },
    Ref(String),
    
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
        }
    }
    
    pub fn with_base_path(mut self, path: std::path::PathBuf) -> Self {
        self.base_path = Some(path);
        self
    }

    pub fn parse(&mut self) -> Result<Document> {
        let blocks = self.parse_blocks()?;

        self.load_bibliography_files();
        let bibliography = self.render_bibliography();

        Ok(Document {
            metadata: self.metadata.clone(),
            blocks,
            footnotes: self.state.footnote_texts.clone(),
            bibliography,
        })
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
                
                // Check for label
                let label = if matches!(self.current_token(), Token::Label(_)) {
                    if let Token::Label(l) = self.current_token() {
                        let label = Some(l.clone());
                        self.advance();
                        label
                    } else {
                        None
                    }
                } else {
                    None
                };

                if let Some(section_label) = &label {
                    self.state.add_label(
                        section_label.clone(),
                        LabelType::Section,
                        Some(title_text.clone()),
                    );
                }

                Ok(Some(Block::Section {
                    level,
                    title: vec![Inline::Text(title_text)],
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
                Ok(Some(Block::DisplayMath(content)))
            }
            Token::Text(_) | Token::InlineMath(_) | Token::LeftBrace => {
                self.parse_paragraph()
            }
            _ => {
                self.advance();
                Ok(None)
            }
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
            "quote" | "quotation" => self.parse_quote_environment(),
            "verbatim" => self.parse_verbatim(),
            "lstlisting" | "minted" => self.parse_code_block(&env_name),
            "equation" | "equation*" | "align" | "align*" | "gather" | "gather*" => {
                self.parse_math_environment(&env_name)
            }
            "table" => self.parse_table_environment(),
            "tabular" => self.parse_tabular(),
            "figure" => self.parse_figure_environment(),
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
                // Preserve unsupported environments as raw LaTeX.
                let content = self.read_until_end_environment(&env_name)?;
                Ok(Some(Block::RawBlock(format!(
                    "\\begin{{{}}}{}\\end{{{}}}",
                    env_name, content, env_name
                ))))
            }
        }
    }

    fn parse_document_environment(&mut self) -> Result<Option<Block>> {
        // Parse document environment contents as blocks
        while !self.is_at_end() {
            self.skip_whitespace_and_comments();
            
            if let Token::EndEnvironment(env) = self.current_token() {
                if env == "document" {
                    self.advance();
                    break;
                }
            }
            
            // Don't consume tokens here - let parse_block handle them
            break;
        }
        Ok(None)
    }

    fn parse_itemize(&mut self) -> Result<Option<Block>> {
        let mut items = Vec::new();

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
                let item_blocks = self.parse_item_content("itemize")?;
                items.push(item_blocks);
            } else {
                self.advance();
            }
        }

        Ok(Some(Block::BulletList(items)))
    }

    fn parse_enumerate(&mut self) -> Result<Option<Block>> {
        let mut items = Vec::new();

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
                Token::ParBreak | Token::BeginEnvironment(_) | Token::DisplayMath(_) => break,
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

    fn parse_quote_environment(&mut self) -> Result<Option<Block>> {
        let mut blocks = Vec::new();

        while !self.is_at_end() {
            self.skip_whitespace_and_comments();

            if let Token::EndEnvironment(env) = self.current_token() {
                if env == "quote" || env == "quotation" {
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
        // Check if environment has a label
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
                if env_name.starts_with("equation") || env_name.starts_with("align") {
                    use crate::state::LabelType;
                    self.state.add_label(label.clone(), LabelType::Equation, None);
                }
                continue;
            }
            
            content.push_str(&self.token_to_string(self.current_token()));
            self.advance();
        }
        
        // For numbered environments, add equation number
        let is_numbered = !env_name.ends_with('*');
        if is_numbered && has_label {
            if let Some(label_info) = self.state.get_label(&label) {
                content = format!("{}\\tag{{{}}}", content.trim(), label_info.number);
            }
        }
        
        Ok(Some(Block::DisplayMath(content.trim().to_string())))
    }

    fn parse_table_environment(&mut self) -> Result<Option<Block>> {
        let mut caption = None;
        let mut tabular_block = None;
        let mut label = None;

        while !self.is_at_end() {
            self.skip_whitespace_and_comments();

            if let Token::EndEnvironment(env) = self.current_token() {
                if env == "table" {
                    self.advance();
                    break;
                }
            }

            if let Token::Command(cmd) = self.current_token() {
                if cmd == "caption" {
                    self.advance();
                    caption = Some(self.parse_braced_inlines()?);
                    continue;
                }
            }

            if let Token::BeginEnvironment(env) = self.current_token() {
                if env == "tabular" {
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

    fn parse_tabular(&mut self) -> Result<Option<Block>> {
        self.advance(); // consume BeginEnvironment

        // Parse column specification
        let alignments = if matches!(self.current_token(), Token::LeftBrace) {
            self.advance();
            let spec = self.read_until_token(Token::RightBrace);
            if matches!(self.current_token(), Token::RightBrace) {
                self.advance();
            }
            self.parse_column_spec(&spec)
        } else {
            vec![Alignment::AlignDefault]
        };

        let mut rows = Vec::new();
        let mut current_row = Vec::new();
        let mut current_cell = Vec::new();

        while !self.is_at_end() {
            self.skip_whitespace_and_comments();

            if let Token::EndEnvironment(env) = self.current_token() {
                if env == "tabular" {
                    self.advance();
                    break;
                }
            }

            match self.current_token() {
                Token::Ampersand => {
                    current_row.push(current_cell);
                    current_cell = Vec::new();
                    self.advance();
                }
                Token::Backslash => {
                    current_row.push(current_cell);
                    rows.push(current_row);
                    current_row = Vec::new();
                    current_cell = Vec::new();
                    self.advance();
                }
                _ => {
                    if let Some(block) = self.parse_block()? {
                        current_cell.push(block);
                    }
                }
            }
        }

        // Handle last cell and row
        if !current_cell.is_empty() {
            current_row.push(current_cell);
        }
        if !current_row.is_empty() {
            rows.push(current_row);
        }

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

    fn parse_column_spec(&self, spec: &str) -> Vec<Alignment> {
        spec.chars()
            .filter_map(|ch| match ch {
                'l' => Some(Alignment::AlignLeft),
                'c' => Some(Alignment::AlignCenter),
                'r' => Some(Alignment::AlignRight),
                _ => None,
            })
            .collect()
    }

    fn parse_figure_environment(&mut self) -> Result<Option<Block>> {
        let mut caption = None;
        let mut path = String::new();
        let mut label = None;

        while !self.is_at_end() {
            self.skip_whitespace_and_comments();

            if let Token::EndEnvironment(env) = self.current_token() {
                if env == "figure" {
                    self.advance();
                    break;
                }
            }

            if let Token::Command(cmd) = self.current_token() {
                match cmd.as_str() {
                    "includegraphics" => {
                        self.advance();
                        // Skip optional arguments
                        if matches!(self.current_token(), Token::LeftBracket) {
                            self.advance();
                            let _ = self.read_until_token(Token::RightBracket);
                            if matches!(self.current_token(), Token::RightBracket) {
                                self.advance();
                            }
                        }
                        // Get file path
                        if matches!(self.current_token(), Token::LeftBrace) {
                            self.advance();
                            path = self.read_until_token(Token::RightBrace);
                            if matches!(self.current_token(), Token::RightBrace) {
                                self.advance();
                            }
                        }
                    }
                    "caption" => {
                        self.advance();
                        caption = Some(self.parse_braced_inlines()?);
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

        Ok(Some(Block::Figure { caption, path, label }))
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
                | Token::DisplayMath(_) => {
                    break;
                }
                Token::EndEnvironment(_) => {
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
                let content = content.clone();
                self.advance();
                Ok(Some(Inline::InlineMath(content)))
            }
            Token::Command(cmd) => self.parse_inline_command(cmd.clone()),
            Token::LeftBrace => self.parse_braced_group(),
            Token::Tilde => {
                self.advance();
                Ok(Some(Inline::Text(String::from(" "))))
            }
            Token::Backslash => {
                self.advance();
                Ok(Some(Inline::LineBreak))
            }
            Token::Ref { kind, label } => {
                let kind = kind.clone();
                let label = label.clone();
                self.advance();
                if let Some(label_info) = self.state.get_label(&label) {
                    let ref_text = match kind.as_str() {
                        "nameref" => label_info.title.clone().unwrap_or_else(|| label.clone()),
                        "autoref" => {
                            let prefix = match &label_info.label_type {
                                LabelType::Section => "Section",
                                LabelType::Figure => "Figure",
                                LabelType::Table => "Table",
                                LabelType::Equation => "Equation",
                                LabelType::Theorem => "Theorem",
                                LabelType::Lemma => "Lemma",
                                LabelType::Definition => "Definition",
                                LabelType::Other(name) => name.as_str(),
                            };
                            if label_info.number.is_empty() {
                                prefix.to_string()
                            } else {
                                format!("{} {}", prefix, label_info.number)
                            }
                        }
                        _ => label_info.number.clone(),
                    };
                    Ok(Some(Inline::Text(ref_text)))
                } else {
                    Ok(Some(Inline::Ref(label)))
                }
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
                let mode = match kind.as_str() {
                    "citet" | "textcite" => CitationMode::Textual,
                    "citep" | "parencite" => CitationMode::Parenthetical,
                    "citeauthor" => CitationMode::Author,
                    "citeyear" => CitationMode::Year,
                    "citeyearpar" => CitationMode::YearPar,
                    "citealt" => CitationMode::Alt,
                    "citealp" => CitationMode::Alp,
                    _ => CitationMode::Normal,
                };
                let citation_text = self
                    .citation_manager
                    .format_citation(&keys, mode, None, None);
                Ok(Some(Inline::Cite {
                    citations: keys,
                    content: vec![Inline::Text(citation_text)],
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
            let content = self.parse_braced_text()?;
            let num = self.state.add_footnote_mark(content.clone());
            self.state.add_footnote_text(num, content);
            return Ok(Some(Inline::Text(format!("[^{}]", num))));
        }

        // Check if it's a user-defined macro
        if self.macro_processor.is_defined(&self.state, &cmd) {
            return self.parse_macro_expansion(&cmd);
        }

        match cmd.as_str() {
            "textbf" | "bf" => {
                let content = self.parse_braced_inlines()?;
                // Preserve trailing space if present
                let result = Inline::Strong(content);
                Ok(Some(result))
            }
            "textit" | "it" | "emph" => {
                let content = self.parse_braced_inlines()?;
                // Preserve trailing space if present
                let result = Inline::Emph(content);
                Ok(Some(result))
            }
            "underline" => {
                let content = self.parse_braced_inlines()?;
                Ok(Some(Inline::Underline(content)))
            }
            "texttt" | "verb" => {
                let content = self.parse_braced_text()?;
                Ok(Some(Inline::Code(content)))
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
            
            // Text color
            "textcolor" => {
                let _color = self.parse_braced_text()?;
                let content = self.parse_braced_inlines()?;
                // For now, just return the content without color
                Ok(Some(Inline::Span {
                    attrs: HashMap::new(),
                    content,
                }))
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
            
            // Strikeout
            "st" | "sout" => {
                let content = self.parse_braced_inlines()?;
                Ok(Some(Inline::Strikeout(content)))
            }
            
            // Today
            "today" => {
                use chrono::Local;
                Ok(Some(Inline::Text(Local::now().format("%B %d, %Y").to_string())))
            }
            
            _ => {
                if let Ok(result) = self.command_registry.handle(&cmd, &[]) {
                    if !result.is_empty() && result != format!("\\{}", cmd) {
                        return Ok(Some(Inline::RawInline(result)));
                    }
                }
                Ok(Some(Inline::Text(format!("\\{}", cmd))))
            }
        }
    }
    
    fn is_citation_command(&self, cmd: &str) -> bool {
        matches!(cmd, "cite" | "citep" | "citet" | "citealt" | "citealp" | 
                      "citeauthor" | "citeyear" | "citeyearpar" |
                      "autocite" | "textcite" | "parencite" | "footcite")
    }
    
    fn is_reference_command(&self, cmd: &str) -> bool {
        matches!(cmd, "ref" | "eqref" | "autoref" | "nameref" | "pageref")
    }
    
    fn parse_citation_command(&mut self, cmd: &str) -> Result<Option<Inline>> {
        // Parse citation arguments
        let args_text = self.parse_braced_text()?;
        let (keys, prenote, postnote) = parse_citation_args(&args_text);
        
        // Determine citation mode
        let mode = match cmd {
            "citet" | "textcite" => CitationMode::Textual,
            "citep" | "parencite" => CitationMode::Parenthetical,
            "citeauthor" => CitationMode::Author,
            "citeyear" => CitationMode::Year,
            "citeyearpar" => CitationMode::YearPar,
            "citealt" => CitationMode::Alt,
            "citealp" => CitationMode::Alp,
            _ => CitationMode::Normal,
        };
        
        // Format citation
        let citation_text = self.citation_manager.format_citation(
            &keys,
            mode,
            prenote.as_deref(),
            postnote.as_deref(),
        );
        
        Ok(Some(Inline::Cite {
            citations: keys,
            content: vec![Inline::Text(citation_text)],
        }))
    }
    
    fn parse_reference_command(&mut self, cmd: &str) -> Result<Option<Inline>> {
        let label = self.parse_braced_text()?;
        
        // Look up label in state
        if let Some(label_info) = self.state.get_label(&label) {
            let ref_text = match cmd {
                "nameref" => label_info.title.clone().unwrap_or_else(|| label.clone()),
                "autoref" => {
                    let prefix = match &label_info.label_type {
                        LabelType::Section => "Section",
                        LabelType::Figure => "Figure",
                        LabelType::Table => "Table",
                        LabelType::Equation => "Equation",
                        LabelType::Theorem => "Theorem",
                        LabelType::Lemma => "Lemma",
                        LabelType::Definition => "Definition",
                        LabelType::Other(name) => name.as_str(),
                    };
                    if label_info.number.is_empty() {
                        prefix.to_string()
                    } else {
                        format!("{} {}", prefix, label_info.number)
                    }
                }
                _ => label_info.number.clone(),
            };
            Ok(Some(Inline::Text(ref_text)))
        } else {
            // Label not found, return placeholder
            Ok(Some(Inline::Ref(label)))
        }
    }
    
    fn parse_macro_expansion(&mut self, cmd: &str) -> Result<Option<Inline>> {
        let args = self.parse_macro_arguments(cmd)?;

        if let Some(expanded) = self.macro_processor.expand_macro(&self.state, cmd, &args) {
            Ok(Some(Inline::Text(expanded)))
        } else {
            Ok(Some(Inline::Text(format!("\\{}", cmd))))
        }
    }

    fn parse_braced_group(&mut self) -> Result<Option<Inline>> {
        self.advance(); // consume '{'
        let inlines = self.parse_inlines_until(Token::RightBrace)?;
        
        if matches!(self.current_token(), Token::RightBrace) {
            self.advance();
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
        if !matches!(self.current_token(), Token::LeftBrace) {
            return Ok(String::new());
        }

        self.advance(); // consume '{'
        let text = self.read_until_token(Token::RightBrace);
        
        if matches!(self.current_token(), Token::RightBrace) {
            self.advance();
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
        let name_raw = self.parse_braced_text()?;
        if name_raw.is_empty() {
            return Ok(());
        }

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
                Inline::Text(value) | Inline::Code(value) | Inline::InlineMath(value) | Inline::RawInline(value) | Inline::Ref(value) => {
                    text.push_str(value);
                }
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
            Token::Command(cmd) => format!("\\{}", cmd),
            Token::LeftBrace => String::from("{"),
            Token::RightBrace => String::from("}"),
            Token::LeftBracket => String::from("["),
            Token::RightBracket => String::from("]"),
            Token::Backslash => String::from("\\\\"),
            Token::Ampersand => String::from("&"),
            Token::Tilde => String::from("~"),
            _ => String::new(),
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
                    let base = self.base_path.as_ref().map(|p| p.as_path());
                    match if cmd == "include" {
                        self.include_system.include_file(&mut self.state, &filename, base)
                    } else {
                        self.include_system.input_file(&mut self.state, &filename, base)
                    } {
                        Ok(content) => self.parse_included_content(&filename, &content),
                        Err(_) => Ok(None), // Silently ignore missing files
                    }
                } else {
                    Ok(None)
                }
            }
            
            // Package loading
            "usepackage" => {
                self.advance();
                // Parse options if present (skip for now)
                if matches!(self.current_token(), Token::LeftBracket) {
                    self.advance();
                    while !self.is_at_end() && !matches!(self.current_token(), Token::RightBracket) {
                        self.advance();
                    }
                    if matches!(self.current_token(), Token::RightBracket) {
                        self.advance();
                    }
                }
                
                let package = self.parse_braced_text()?;
                if !package.is_empty() {
                    // Try to load package
                    let _ = self.include_system.load_package(&mut self.state, &package, &[]);
                }
                Ok(None)
            }
            
            // Macro definitions
            "newcommand" | "renewcommand" | "providecommand" => {
                self.advance();
                let mode = match cmd.as_str() {
                    "renewcommand" => MacroDefinitionMode::Renew,
                    "providecommand" => MacroDefinitionMode::Provide,
                    _ => MacroDefinitionMode::New,
                };
                self.parse_macro_definition_command(mode)?;
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
                // Skip bibliography style
                let _ = self.parse_braced_text()?;
                Ok(None)
            }
            
            // Metadata commands
            "title" => {
                self.advance();
                let title = self.parse_braced_text()?;
                if !title.is_empty() {
                    self.metadata.insert("title".to_string(), title);
                }
                Ok(None)
            }
            "author" => {
                self.advance();
                // Parse author as inlines to handle \thanks footnotes
                let author_inlines = self.parse_braced_inlines()?;
                if !author_inlines.is_empty() {
                    // Convert inlines to text, extracting footnotes
                    let mut author_text = String::new();
                    for inline in author_inlines {
                        match inline {
                            Inline::Text(t) => author_text.push_str(&t),
                            Inline::Space => author_text.push(' '),
                            _ => {}
                        }
                    }
                    if !author_text.is_empty() {
                        self.metadata.insert("author".to_string(), author_text.trim().to_string());
                    }
                }
                Ok(None)
            }
            "date" => {
                self.advance();
                let date = self.parse_braced_text()?;
                if !date.is_empty() {
                    self.metadata.insert("date".to_string(), date);
                }
                Ok(None)
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
