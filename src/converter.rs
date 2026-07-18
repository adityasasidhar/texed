use crate::error::Result;
use crate::parser::{Alignment, Block, Document, Inline, QuoteType};
use std::fmt::Write;

pub struct MarkdownConverter {
    output: String,
    in_table: bool,
    frontmatter: bool,
}

impl MarkdownConverter {
    pub fn new() -> Self {
        Self {
            output: String::new(),
            in_table: false,
            frontmatter: true,
        }
    }

    /// Disable the YAML frontmatter block (title/author/date metadata).
    pub fn with_frontmatter(mut self, enabled: bool) -> Self {
        self.frontmatter = enabled;
        self
    }

    /// Render a slice of inlines to a standalone Markdown string.
    /// Used by the parser for footnote texts, metadata values, etc.
    pub fn render_inlines_fragment(inlines: &[Inline]) -> String {
        let mut converter = Self::new();
        let _ = converter.convert_inlines(inlines);
        converter.output.trim().to_string()
    }

    pub fn convert(&mut self, document: Document) -> Result<String> {
        // Add metadata as YAML frontmatter if present
        if self.frontmatter && !document.metadata.is_empty() {
            self.output.push_str("---\n");
            // Stable, reader-friendly key order
            let mut keys: Vec<&String> = document.metadata.keys().collect();
            keys.sort_by_key(|key| match key.as_str() {
                "title" => (0, key.as_str()),
                "author" => (1, key.as_str()),
                "date" => (2, key.as_str()),
                other => (3, other),
            });
            for key in keys {
                let value = &document.metadata[key];
                writeln!(self.output, "{}: {}", key, yaml_scalar(value)).unwrap();
            }
            self.output.push_str("---\n\n");
        }

        // Convert blocks
        for block in document.blocks {
            self.convert_block(&block)?;
        }

        // Add footnotes at the end if any exist
        if !document.footnotes.is_empty() {
            self.output.push_str("\n---\n\n");
            let mut numbers: Vec<&usize> = document.footnotes.keys().collect();
            numbers.sort();
            for num in numbers {
                writeln!(self.output, "[^{}]: {}", num, document.footnotes[num]).unwrap();
            }
        }

        if let Some(bibliography) = &document.bibliography {
            if !self.output.ends_with("\n\n") {
                self.output.push('\n');
            }
            self.output.push_str(bibliography);
        }

        Ok(self.output.clone())
    }

    fn convert_block(&mut self, block: &Block) -> Result<()> {
        match block {
            Block::Section { level, title, label } => {
                self.convert_section(*level, title, label.as_deref())?;
            }
            Block::Paragraph(inlines) => {
                self.convert_paragraph(inlines)?;
            }
            Block::BulletList(items) => {
                self.convert_bullet_list(items)?;
            }
            Block::OrderedList { start, items } => {
                self.convert_ordered_list(*start, items)?;
            }
            Block::DescriptionList(items) => {
                self.convert_description_list(items)?;
            }
            Block::Quote(blocks) => {
                self.convert_quote(blocks)?;
            }
            Block::CodeBlock { language, content } => {
                self.convert_code_block(language.as_deref(), content)?;
            }
            Block::Verbatim(content) => {
                self.convert_verbatim(content)?;
            }
            Block::DisplayMath(content) => {
                self.convert_display_math(content)?;
            }
            Block::Table { caption, alignments, headers, rows } => {
                self.convert_table(caption.as_ref(), alignments, headers, rows)?;
            }
            Block::Figure { caption, path, label } => {
                self.convert_figure(caption.as_ref(), path, label.as_deref())?;
            }
            Block::Composite(blocks) => {
                for block in blocks {
                    self.convert_block(block)?;
                }
            }
            Block::TheoremLike {
                env_type,
                display_name,
                number,
                label,
                title,
                content,
            } => {
                self.convert_theorem_like(
                    env_type,
                    display_name,
                    number.as_deref(),
                    label.as_deref(),
                    title.as_deref(),
                    content,
                )?;
            }
            Block::RawBlock(content) => {
                self.output.push_str(content);
                self.output.push('\n');
            }
            Block::HorizontalRule => {
                self.output.push_str("---\n\n");
            }
            Block::Null => {}
        }

        Ok(())
    }

    fn convert_section(&mut self, level: u8, title: &[Inline], label: Option<&str>) -> Result<()> {
        // H1 is reserved for the document title (\maketitle / \chapter / \part),
        // so \section (level 1) maps to ##, \subsection to ###, and so on.
        let md_level = (level + 1).min(6);

        for _ in 0..md_level {
            self.output.push('#');
        }
        self.output.push(' ');

        let heading = self.render_inlines_trimmed(title)?;
        self.output.push_str(&heading);

        if let Some(label) = label {
            write!(self.output, " {{#{}}}", label).unwrap();
        }

        self.output.push_str("\n\n");
        Ok(())
    }

    fn convert_paragraph(&mut self, inlines: &[Inline]) -> Result<()> {
        if inlines.is_empty() {
            return Ok(());
        }

        let text = self.render_inlines_trimmed(inlines)?;
        if !text.is_empty() {
            self.output.push_str(&text);
            self.output.push_str("\n\n");
        }
        Ok(())
    }

    fn convert_bullet_list(&mut self, items: &[Vec<Block>]) -> Result<()> {
        for item in items {
            self.convert_list_item("- ", item)?;
        }
        self.output.push('\n');
        Ok(())
    }

    fn convert_ordered_list(&mut self, start: usize, items: &[Vec<Block>]) -> Result<()> {
        for (i, item) in items.iter().enumerate() {
            let marker = format!("{}. ", start + i);
            self.convert_list_item(&marker, item)?;
        }
        self.output.push('\n');
        Ok(())
    }

    /// Render one list item: first line after the marker, continuation lines
    /// (further paragraphs, nested lists, code blocks) indented to align.
    fn convert_list_item(&mut self, marker: &str, blocks: &[Block]) -> Result<()> {
        let saved_output = std::mem::take(&mut self.output);

        for (i, block) in blocks.iter().enumerate() {
            // A nested list directly after the item text stays tight
            if i > 0
                && matches!(block, Block::BulletList(_) | Block::OrderedList { .. })
                && matches!(blocks[i - 1], Block::Paragraph(_))
            {
                while self.output.ends_with('\n') {
                    self.output.pop();
                }
                self.output.push('\n');
            }
            self.convert_block(block)?;
        }

        let item_content = self.output.trim_end().to_string();
        self.output = saved_output;

        let indent = " ".repeat(marker.len());
        self.output.push_str(marker);

        let mut lines = item_content.lines();
        match lines.next() {
            Some(first) => {
                self.output.push_str(first);
                self.output.push('\n');
            }
            None => {
                self.output.push('\n');
                return Ok(());
            }
        }
        for line in lines {
            if line.is_empty() {
                self.output.push('\n');
            } else {
                self.output.push_str(&indent);
                self.output.push_str(line);
                self.output.push('\n');
            }
        }
        Ok(())
    }

    fn convert_description_list(&mut self, items: &[(Vec<Inline>, Vec<Block>)]) -> Result<()> {
        for (term, description) in items {
            // Term in bold
            self.output.push_str("**");
            self.convert_inlines(term)?;
            self.output.push_str("**\n");

            // Description indented
            for block in description {
                self.output.push_str(": ");
                
                let saved_output = self.output.clone();
                self.output.clear();
                self.convert_block(block)?;
                let desc_content = self.output.trim_end().to_string();
                self.output = saved_output;

                self.output.push_str(&desc_content);
                self.output.push('\n');
            }

            self.output.push('\n');
        }

        Ok(())
    }

    fn convert_quote(&mut self, blocks: &[Block]) -> Result<()> {
        let saved_output = self.output.clone();
        self.output.clear();

        for block in blocks {
            self.convert_block(block)?;
        }

        let quote_content = self.output.trim_end().to_string();
        self.output = saved_output;

        for line in quote_content.lines() {
            if line.is_empty() {
                self.output.push_str(">\n");
            } else {
                self.output.push_str("> ");
                self.output.push_str(line);
                self.output.push('\n');
            }
        }

        self.output.push('\n');
        Ok(())
    }

    fn convert_code_block(&mut self, language: Option<&str>, content: &str) -> Result<()> {
        self.output.push_str("```");
        if let Some(lang) = language {
            self.output.push_str(lang);
        }
        self.output.push('\n');
        self.output.push_str(content.trim_end());
        self.output.push_str("\n```\n\n");
        Ok(())
    }

    fn convert_verbatim(&mut self, content: &str) -> Result<()> {
        self.convert_code_block(None, content)
    }

    fn convert_display_math(&mut self, content: &str) -> Result<()> {
        self.output.push_str("$$\n");
        self.output.push_str(content.trim());
        self.output.push_str("\n$$\n\n");
        Ok(())
    }

    fn convert_table(
        &mut self,
        caption: Option<&Vec<Inline>>,
        alignments: &[Alignment],
        headers: &[Vec<Block>],
        rows: &[Vec<Vec<Block>>],
    ) -> Result<()> {
        self.in_table = true;

        // Caption before table
        if let Some(cap) = caption {
            let text = self.render_inlines_trimmed(cap)?;
            if !text.is_empty() {
                self.output.push_str("**");
                self.output.push_str(&text);
                self.output.push_str("**\n\n");
            }
        }

        // Render every cell up front so columns can be padded for readability
        let header_cells: Vec<String> = headers
            .iter()
            .map(|cell| self.render_table_cell(cell))
            .collect::<Result<_>>()?;
        let body_cells: Vec<Vec<String>> = rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| self.render_table_cell(cell))
                    .collect::<Result<_>>()
            })
            .collect::<Result<_>>()?;

        self.in_table = false;

        let num_columns = header_cells
            .len()
            .max(body_cells.iter().map(|row| row.len()).max().unwrap_or(0));
        if num_columns == 0 {
            return Ok(());
        }

        let mut widths = vec![3usize; num_columns];
        for (i, cell) in header_cells.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
        for row in &body_cells {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(cell.chars().count());
            }
        }

        let alignment_of = |i: usize| alignments.get(i).unwrap_or(&Alignment::AlignDefault);

        let write_row = |output: &mut String, cells: &[String]| {
            output.push('|');
            for (i, width) in widths.iter().enumerate() {
                let empty = String::new();
                let cell = cells.get(i).unwrap_or(&empty);
                let pad = width.saturating_sub(cell.chars().count());
                match alignment_of(i) {
                    Alignment::AlignRight => {
                        output.push(' ');
                        for _ in 0..pad {
                            output.push(' ');
                        }
                        output.push_str(cell);
                        output.push_str(" |");
                    }
                    Alignment::AlignCenter => {
                        let left = pad / 2;
                        output.push(' ');
                        for _ in 0..left {
                            output.push(' ');
                        }
                        output.push_str(cell);
                        for _ in 0..(pad - left) {
                            output.push(' ');
                        }
                        output.push_str(" |");
                    }
                    _ => {
                        output.push(' ');
                        output.push_str(cell);
                        for _ in 0..pad {
                            output.push(' ');
                        }
                        output.push_str(" |");
                    }
                }
            }
            output.push('\n');
        };

        // GFM requires a header row; fall back to an empty one
        write_row(&mut self.output, &header_cells);

        self.output.push('|');
        for (i, &width) in widths.iter().enumerate() {
            let bar = match alignment_of(i) {
                Alignment::AlignLeft => format!(":{}", "-".repeat(width + 1)),
                Alignment::AlignRight => format!("{}:", "-".repeat(width + 1)),
                Alignment::AlignCenter => format!(":{}:", "-".repeat(width)),
                Alignment::AlignDefault => "-".repeat(width + 2),
            };
            self.output.push_str(&bar);
            self.output.push('|');
        }
        self.output.push('\n');

        for row in &body_cells {
            write_row(&mut self.output, row);
        }

        self.output.push('\n');
        Ok(())
    }

    fn render_table_cell(&mut self, blocks: &[Block]) -> Result<String> {
        let saved_output = std::mem::take(&mut self.output);

        for block in blocks {
            self.convert_block(block)?;
        }

        let cell_content = self.output.trim().replace('\n', " ");
        self.output = saved_output;
        Ok(cell_content)
    }

    fn convert_figure(&mut self, caption: Option<&Vec<Inline>>, path: &str, label: Option<&str>) -> Result<()> {
        self.output.push_str("![");
        
        if let Some(cap) = caption {
            self.convert_inlines(cap)?;
        }
        
        self.output.push_str("](");
        self.output.push_str(path);
        self.output.push(')');

        if let Some(label) = label {
            write!(self.output, " {{#{}}}", label).unwrap();
        }

        self.output.push_str("\n\n");
        Ok(())
    }

    fn convert_theorem_like(
        &mut self,
        env_type: &str,
        display_name: &str,
        number: Option<&str>,
        label: Option<&str>,
        title: Option<&str>,
        content: &[Block],
    ) -> Result<()> {
        // Convert to blockquote with bold header
        self.output.push_str("> **");
        if display_name.is_empty() {
            self.output.push_str(&env_type.to_uppercase());
        } else {
            self.output.push_str(display_name);
        }
        if let Some(n) = number {
            self.output.push(' ');
            self.output.push_str(n);
        }
        
        if let Some(t) = title {
            self.output.push_str(" (");
            self.output.push_str(t);
            self.output.push(')');
        }

        self.output.push_str("**");

        if let Some(label) = label {
            write!(self.output, " {{#{}}}", label).unwrap();
        }

        self.output.push_str("\n>\n");

        // Convert content as quoted
        let saved_output = self.output.clone();
        self.output.clear();

        for block in content {
            self.convert_block(block)?;
        }

        let theorem_content = self.output.trim_end().to_string();
        self.output = saved_output;

        for line in theorem_content.lines() {
            self.output.push_str("> ");
            self.output.push_str(line);
            self.output.push('\n');
        }

        self.output.push('\n');
        Ok(())
    }

    fn convert_inlines(&mut self, inlines: &[Inline]) -> Result<()> {
        for inline in inlines {
            self.convert_inline(inline)?;
        }
        Ok(())
    }

    fn render_inlines_trimmed(&mut self, inlines: &[Inline]) -> Result<String> {
        let saved_output = self.output.clone();
        self.output.clear();
        self.convert_inlines(inlines)?;
        let text = self.output.trim().to_string();
        self.output = saved_output;
        Ok(text)
    }

    fn convert_inline(&mut self, inline: &Inline) -> Result<()> {
        match inline {
            Inline::Text(text) => {
                let escaped = escape_markdown_text(text, self.in_table);
                self.output.push_str(&escaped);
            }
            Inline::Space => {
                // Collapse runs of whitespace
                if !self.output.is_empty()
                    && !self.output.ends_with(' ')
                    && !self.output.ends_with('\n')
                {
                    self.output.push(' ');
                }
            }
            Inline::SoftBreak => {
                self.output.push('\n');
            }
            Inline::LineBreak => {
                if self.in_table {
                    self.output.push_str("<br>");
                } else {
                    self.output.push_str("  \n");
                }
            }
            Inline::Emph(content) => {
                self.output.push('*');
                self.convert_inlines(content)?;
                self.output.push('*');
            }
            Inline::Strong(content) => {
                self.output.push_str("**");
                self.convert_inlines(content)?;
                self.output.push_str("**");
            }
            Inline::Strikeout(content) => {
                self.output.push_str("~~");
                self.convert_inlines(content)?;
                self.output.push_str("~~");
            }
            Inline::Underline(content) => {
                // Markdown doesn't have native underline, use HTML
                self.output.push_str("<u>");
                self.convert_inlines(content)?;
                self.output.push_str("</u>");
            }
            Inline::Superscript(content) => {
                self.output.push_str("<sup>");
                self.convert_inlines(content)?;
                self.output.push_str("</sup>");
            }
            Inline::Subscript(content) => {
                self.output.push_str("<sub>");
                self.convert_inlines(content)?;
                self.output.push_str("</sub>");
            }
            Inline::SmallCaps(content) => {
                // Small caps using CSS
                self.output.push_str("<span style=\"font-variant: small-caps;\">");
                self.convert_inlines(content)?;
                self.output.push_str("</span>");
            }
            Inline::Code(code) => {
                let code = if self.in_table {
                    code.replace('|', "\\|")
                } else {
                    code.clone()
                };
                // Code containing backticks needs a longer fence and padding
                if code.contains('`') {
                    self.output.push_str("`` ");
                    self.output.push_str(&code);
                    self.output.push_str(" ``");
                } else {
                    self.output.push('`');
                    self.output.push_str(&code);
                    self.output.push('`');
                }
            }
            Inline::InlineMath(math) => {
                let math = if self.in_table {
                    math.replace('|', "\\|")
                } else {
                    math.clone()
                };
                self.output.push('$');
                self.output.push_str(math.trim());
                self.output.push('$');
            }
            Inline::Link { text, url, title } => {
                self.output.push('[');
                self.convert_inlines(text)?;
                self.output.push_str("](");
                self.output.push_str(url);
                if let Some(t) = title {
                    self.output.push_str(" \"");
                    self.output.push_str(t);
                    self.output.push('"');
                }
                self.output.push(')');
            }
            Inline::Image { alt, url, title } => {
                self.output.push_str("![");
                self.convert_inlines(alt)?;
                self.output.push_str("](");
                self.output.push_str(url);
                if let Some(t) = title {
                    self.output.push_str(" \"");
                    self.output.push_str(t);
                    self.output.push('"');
                }
                self.output.push(')');
            }
            Inline::Cite {
                citations, content, ..
            } => {
                if !content.is_empty() {
                    self.convert_inlines(content)?;
                } else {
                    self.output.push('[');
                    for (i, citation) in citations.iter().enumerate() {
                        if i > 0 {
                            self.output.push_str(", ");
                        }
                        self.output.push('@');
                        self.output.push_str(citation);
                    }
                    self.output.push(']');
                }
            }
            Inline::Ref { label, .. } => {
                self.output.push('[');
                self.output.push_str(label);
                self.output.push_str("](#");
                self.output.push_str(label);
                self.output.push(')');
            }
            Inline::RawInline(content) => {
                self.output.push_str(content);
            }
            Inline::Note(blocks) => {
                self.output.push_str("[^note]: ");
                for block in blocks {
                    self.convert_block(block)?;
                }
            }
            Inline::Span { content, .. } => {
                self.convert_inlines(content)?;
            }
            Inline::Quoted { quote_type, content } => {
                match quote_type {
                    QuoteType::SingleQuote => {
                        self.output.push('\'');
                        self.convert_inlines(content)?;
                        self.output.push('\'');
                    }
                    QuoteType::DoubleQuote => {
                        self.output.push('"');
                        self.convert_inlines(content)?;
                        self.output.push('"');
                    }
                }
            }
        }

        Ok(())
    }
}

impl Default for MarkdownConverter {
    fn default() -> Self {
        Self::new()
    }
}

/// Escape characters that would otherwise be interpreted as Markdown syntax.
/// Kept minimal: only characters that realistically appear in LaTeX text and
/// change Markdown semantics.
fn escape_markdown_text(text: &str, in_table: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '*' | '_' | '`' | '$' => {
                out.push('\\');
                out.push(ch);
            }
            '|' if in_table => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Quote a YAML scalar when needed.
fn yaml_scalar(value: &str) -> String {
    let needs_quoting = value.is_empty()
        || value.contains(": ")
        || value.ends_with(':')
        || value.contains('#')
        || value.contains('"')
        || value.starts_with(['\'', '"', '&', '*', '?', '|', '-', '<', '>', '=', '!', '%', '@', '`', '[', ']', '{', '}'])
        || value.starts_with(' ')
        || value.ends_with(' ');
    if needs_quoting {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn convert(latex: &str) -> String {
        let mut lexer = Lexer::new(latex);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let doc = parser.parse().unwrap();
        let mut converter = MarkdownConverter::new();
        converter.convert(doc).unwrap()
    }

    #[test]
    fn test_convert_paragraph() {
        assert!(convert("Hello world").contains("Hello world"));
    }

    #[test]
    fn test_convert_section() {
        assert!(convert("\\section{Introduction}").contains("## Introduction"));
    }

    #[test]
    fn test_heading_levels() {
        let md = convert("\\chapter{C}\n\\section{S}\n\\subsection{SS}\n\\subsubsection{SSS}\n\\paragraph{P}");
        assert!(md.contains("# C"));
        assert!(md.contains("## S"));
        assert!(md.contains("### SS"));
        assert!(md.contains("#### SSS"));
        assert!(md.contains("##### P"));
    }

    #[test]
    fn test_convert_emphasis() {
        let markdown = convert("\\textit{italic} and \\textbf{bold}");
        assert!(markdown.contains("*italic*"));
        assert!(markdown.contains("**bold**"));
    }

    #[test]
    fn test_section_title_with_formatting_and_label() {
        let md = convert("\\section{The \\texttt{code} story\\label{sec:x}}\nSee \\ref{sec:x}.");
        assert!(md.contains("## The `code` story {#sec:x}"), "got: {}", md);
        assert!(md.contains("See 1."), "got: {}", md);
    }

    #[test]
    fn test_escaped_special_characters() {
        let md = convert("50\\% of R\\&D \\$5 \\#1 a\\_b \\{x\\}");
        assert!(md.contains("50% of R&D"), "got: {}", md);
        assert!(md.contains("\\$5"), "got: {}", md);
        assert!(md.contains("#1"), "got: {}", md);
        assert!(md.contains("a\\_b"), "got: {}", md);
        assert!(md.contains("{x}"), "got: {}", md);
    }

    #[test]
    fn test_escaped_percent_does_not_eat_line() {
        let md = convert("Save 10\\% today and tomorrow.");
        assert!(md.contains("Save 10% today and tomorrow."), "got: {}", md);
    }

    #[test]
    fn test_accents() {
        let md = convert("Caf\\'e na\\\"ive fa\\c{c}ade Erd\\H{o}s \\v{S}koda \\~nino");
        assert!(md.contains("Café"), "got: {}", md);
        assert!(md.contains("naïve"), "got: {}", md);
        assert!(md.contains("façade"), "got: {}", md);
        assert!(md.contains("Erdős"), "got: {}", md);
        assert!(md.contains("ñino"), "got: {}", md);
    }

    #[test]
    fn test_smart_quotes_and_dashes() {
        let md = convert("``double'' `single' a--b c---d");
        assert!(md.contains("\u{201C}double\u{201D}"), "got: {}", md);
        assert!(md.contains("\u{2018}single\u{2019}"), "got: {}", md);
        assert!(md.contains("a\u{2013}b"), "got: {}", md);
        assert!(md.contains("c\u{2014}d"), "got: {}", md);
    }

    #[test]
    fn test_special_char_commands() {
        let md = convert("\\ldots\\ \\S5 \\copyright\\ \\ss\\ \\ae");
        assert!(md.contains("\u{2026}"), "got: {}", md);
        assert!(md.contains("§5"), "got: {}", md);
        assert!(md.contains("©"), "got: {}", md);
        assert!(md.contains("ß"), "got: {}", md);
        assert!(md.contains("æ"), "got: {}", md);
    }

    #[test]
    fn test_verb_command() {
        let md = convert("Use \\verb|x_1 & y| here and \\verb+a|b+ too.");
        assert!(md.contains("`x_1 & y`"), "got: {}", md);
        assert!(md.contains("`a|b`"), "got: {}", md);
    }

    #[test]
    fn test_verbatim_preserves_latex_specials() {
        let md = convert("\\begin{verbatim}\n% not a comment $x$ \\cmd\n\\end{verbatim}");
        assert!(md.contains("% not a comment $x$ \\cmd"), "got: {}", md);
    }

    #[test]
    fn test_lstlisting_language() {
        let md = convert("\\begin{lstlisting}[language=Python]\nprint(1)\n\\end{lstlisting}");
        assert!(md.contains("```python"), "got: {}", md);
        assert!(md.contains("print(1)"), "got: {}", md);
    }

    #[test]
    fn test_table_with_rules_and_alignment() {
        let md = convert(
            "\\begin{tabular}{lcr}\n\\hline\nA & B & C \\\\\n\\hline\n1 & 2 & 3 \\\\\n\\hline\n\\end{tabular}",
        );
        assert!(md.contains("| A"), "got: {}", md);
        assert!(md.contains("| 1"), "got: {}", md);
        assert!(md.contains(":--"), "got: {}", md);
        assert!(md.contains("--:"), "got: {}", md);
        assert!(!md.contains("hline"), "got: {}", md);
    }

    #[test]
    fn test_table_multicolumn_pads_cells() {
        let md = convert(
            "\\begin{tabular}{lll}\nA & B & C \\\\\n\\multicolumn{2}{c}{span} & z \\\\\n\\end{tabular}",
        );
        let span_row = md.lines().find(|l| l.contains("span")).unwrap();
        assert_eq!(span_row.matches('|').count(), 4, "got: {}", md);
    }

    #[test]
    fn test_nested_lists_are_tight() {
        let md = convert(
            "\\begin{itemize}\n\\item Top\n\\begin{itemize}\n\\item Nested\n\\end{itemize}\n\\item Next\n\\end{itemize}",
        );
        assert!(md.contains("- Top\n  - Nested\n- Next"), "got: {}", md);
    }

    #[test]
    fn test_align_env_wrapped_for_renderers() {
        let md = convert("\\begin{align*}\na &= b \\\\\nc &= d\n\\end{align*}");
        assert!(md.contains("\\begin{aligned}"), "got: {}", md);
        assert!(md.contains("a &= b"), "got: {}", md);
    }

    #[test]
    fn test_equation_label_becomes_tag_and_eqref_resolves() {
        let md = convert(
            "\\begin{equation}\\label{eq:1}\nx = y\n\\end{equation}\nBy \\eqref{eq:1} we win.",
        );
        assert!(md.contains("\\tag{1}"), "got: {}", md);
        assert!(md.contains("By (1) we win."), "got: {}", md);
    }

    #[test]
    fn test_forward_references_resolve() {
        let md = convert("See \\ref{sec:later} now.\n\\section{Later}\\label{sec:later}");
        assert!(md.contains("See 1 now."), "got: {}", md);
    }

    #[test]
    fn test_macro_with_formatting_body() {
        let md = convert("\\newcommand{\\hi}[1]{\\textbf{hi #1}}\n\\hi{there}");
        assert!(md.contains("**hi there**"), "got: {}", md);
    }

    #[test]
    fn test_math_macro_expansion() {
        let md = convert("\\newcommand{\\R}{\\mathbb{R}}\nSet $\\R^n$ here.");
        assert!(md.contains("$\\mathbb{R}^n$"), "got: {}", md);
    }

    #[test]
    fn test_declaration_groups() {
        let md = convert("{\\bf bold run} and {\\em emphasis run}");
        assert!(md.contains("**bold run**"), "got: {}", md);
        assert!(md.contains("*emphasis run*"), "got: {}", md);
    }

    #[test]
    fn test_noop_commands_are_silent() {
        let md = convert("\\centering\\vspace{1em}\\hspace*{2cm}Text\\clearpage done.\\pagestyle{empty}");
        assert!(md.contains("Text"), "got: {}", md);
        assert!(md.contains("done."), "got: {}", md);
        assert!(!md.contains("\\centering"), "got: {}", md);
        assert!(!md.contains("1em"), "got: {}", md);
        assert!(!md.contains("2cm"), "got: {}", md);
        assert!(!md.contains("empty"), "got: {}", md);
    }

    #[test]
    fn test_maketitle_renders_title_block() {
        let md = convert("\\title{My Paper}\n\\author{A \\and B}\n\\date{2026}\n\\maketitle");
        assert!(md.contains("# My Paper"), "got: {}", md);
        assert!(md.contains("A, B"), "got: {}", md);
        assert!(md.contains("2026"), "got: {}", md);
    }

    #[test]
    fn test_footnote_with_formatting() {
        let md = convert("Claim\\footnote{See \\textit{elsewhere}.} stands.");
        assert!(md.contains("Claim[^1] stands."), "got: {}", md);
        assert!(md.contains("[^1]: See *elsewhere*."), "got: {}", md);
    }

    #[test]
    fn test_thebibliography() {
        let md = convert(
            "As \\cite{a1} shows.\n\\begin{thebibliography}{9}\n\\bibitem{a1} Author One. Title. 2020.\n\\end{thebibliography}",
        );
        assert!(md.contains("As [1] shows."), "got: {}", md);
        assert!(md.contains("## References"), "got: {}", md);
        assert!(md.contains("[1] Author One. Title. 2020."), "got: {}", md);
    }

    #[test]
    fn test_inline_math_paren_syntax() {
        let md = convert("Math \\(a+b\\) inline.");
        assert!(md.contains("$a+b$"), "got: {}", md);
    }

    #[test]
    fn test_center_environment_is_transparent() {
        let md = convert("\\begin{center}\nCentered text\n\\end{center}");
        assert!(md.contains("Centered text"), "got: {}", md);
        assert!(!md.contains("\\begin{center}"), "got: {}", md);
    }

    #[test]
    fn test_comment_at_line_end_joins_words() {
        let md = convert("super% comment\nglue");
        assert!(md.contains("superglue"), "got: {}", md);
    }

    #[test]
    fn test_paragraph_break_preserved_around_full_line_comment() {
        let md = convert("one\n\n% comment\n\ntwo");
        assert!(md.contains("one\n\ntwo"), "got: {}", md);
    }

    #[test]
    fn test_nonbreaking_space() {
        let md = convert("Figure~1");
        assert!(md.contains("Figure\u{00A0}1"), "got: {}", md);
    }

    #[test]
    fn test_newenvironment_expansion() {
        let md = convert(
            "\\newenvironment{note}[1]{\\begin{quote}\\textbf{Note (#1):} }{\\end{quote}}\n\\begin{note}{key}\nBody text.\n\\end{note}",
        );
        assert!(md.contains("> **Note (key):** Body text."), "got: {}", md);
    }

    #[test]
    fn test_newenvironment_empty_defs_passthrough() {
        let md = convert("\\newenvironment{boxed}{}{}\n\\begin{boxed}\nInside.\n\\end{boxed}");
        assert!(md.contains("Inside."), "got: {}", md);
        assert!(!md.contains("\\begin{boxed}"), "got: {}", md);
    }
}
