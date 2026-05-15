use crate::error::Result;
use crate::parser::{Alignment, Block, Document, Inline, QuoteType};
use std::fmt::Write;

pub struct MarkdownConverter {
    output: String,
    list_depth: usize,
    in_table: bool,
}

impl MarkdownConverter {
    pub fn new() -> Self {
        Self {
            output: String::new(),
            list_depth: 0,
            in_table: false,
        }
    }

    pub fn convert(&mut self, document: Document) -> Result<String> {
        // Add metadata as YAML frontmatter if present
        if !document.metadata.is_empty() {
            self.output.push_str("---\n");
            for (key, value) in &document.metadata {
                writeln!(self.output, "{}: {}", key, value).unwrap();
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
            for (num, text) in &document.footnotes {
                writeln!(self.output, "[^{}]: {}", num, text).unwrap();
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
        // Convert level to markdown heading level (1-6)
        // LaTeX \section is level 1, should map to # (md_level 1)
        let md_level = level.min(6);

        for _ in 0..md_level {
            self.output.push('#');
        }
        self.output.push(' ');

        self.convert_inlines(title)?;

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
        self.list_depth += 1;

        for item in items {
            let indent = "  ".repeat(self.list_depth - 1);
            self.output.push_str(&indent);
            self.output.push_str("- ");

            // Convert item blocks inline
            for (i, block) in item.iter().enumerate() {
                if i > 0 {
                    // Add proper indentation for continuation
                    self.output.push_str(&indent);
                    self.output.push_str("  ");
                }
                
                match block {
                    Block::Paragraph(inlines) => {
                        let text = self.render_inlines_trimmed(inlines)?;
                        self.output.push_str(&text);
                    }
                    _ => {
                        self.convert_block(block)?;
                    }
                }
            }
            
            self.output.push('\n');
        }

        self.list_depth -= 1;
        self.output.push('\n');
        Ok(())
    }

    fn convert_ordered_list(&mut self, start: usize, items: &[Vec<Block>]) -> Result<()> {
        self.list_depth += 1;

        for (i, item) in items.iter().enumerate() {
            let indent = "  ".repeat(self.list_depth - 1);
            self.output.push_str(&indent);
            write!(self.output, "{}. ", start + i).unwrap();

            // Convert item blocks
            let saved_output = self.output.clone();
            self.output.clear();

            for block in item {
                self.convert_block(block)?;
            }

            let item_content = self.output.trim_end().to_string();
            self.output = saved_output;

            // Handle multi-line items
            let lines: Vec<&str> = item_content.lines().collect();
            if let Some(first) = lines.first() {
                self.output.push_str(first);
                self.output.push('\n');

                for line in lines.iter().skip(1) {
                    self.output.push_str(&indent);
                    self.output.push_str("   ");
                    self.output.push_str(line);
                    self.output.push('\n');
                }
            }
        }

        self.list_depth -= 1;
        self.output.push('\n');
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
            self.output.push_str("> ");
            self.output.push_str(line);
            self.output.push('\n');
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
            self.output.push_str("**");
            self.convert_inlines(cap)?;
            self.output.push_str("**\n\n");
        }

        // Headers
        if !headers.is_empty() {
            self.output.push('|');
            for cell in headers {
                self.output.push(' ');
                self.convert_table_cell(cell)?;
                self.output.push_str(" |");
            }
            self.output.push('\n');

            // Separator
            self.output.push('|');
            for (i, _) in headers.iter().enumerate() {
                let align = alignments.get(i).unwrap_or(&Alignment::AlignDefault);
                match align {
                    Alignment::AlignLeft => self.output.push_str(" :--- |"),
                    Alignment::AlignRight => self.output.push_str(" ---: |"),
                    Alignment::AlignCenter => self.output.push_str(" :---: |"),
                    Alignment::AlignDefault => self.output.push_str(" --- |"),
                }
            }
            self.output.push('\n');
        }

        // Rows
        for row in rows {
            self.output.push('|');
            for cell in row {
                self.output.push(' ');
                self.convert_table_cell(cell)?;
                self.output.push_str(" |");
            }
            self.output.push('\n');
        }

        self.output.push('\n');
        self.in_table = false;
        Ok(())
    }

    fn convert_table_cell(&mut self, blocks: &[Block]) -> Result<()> {
        let saved_output = self.output.clone();
        self.output.clear();

        for block in blocks {
            self.convert_block(block)?;
        }

        let cell_content = self.output.trim().replace('\n', " ");
        self.output = saved_output;
        self.output.push_str(&cell_content);

        Ok(())
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
        
        self.output.push_str("**\n>\n");

        if let Some(label) = label {
            self.output.push_str("> ");
            write!(self.output, "{{#{}}}", label).unwrap();
            self.output.push('\n');
            self.output.push_str(">\n");
        }

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
                // Escape markdown special characters if not in table
                if self.in_table {
                    self.output.push_str(&text.replace('|', "\\|"));
                } else {
                    self.output.push_str(text);
                }
            }
            Inline::Space => {
                self.output.push(' ');
            }
            Inline::SoftBreak => {
                self.output.push('\n');
            }
            Inline::LineBreak => {
                self.output.push_str("  \n");
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
                self.output.push('`');
                self.output.push_str(code);
                self.output.push('`');
            }
            Inline::InlineMath(math) => {
                self.output.push('$');
                self.output.push_str(math);
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
            Inline::Cite { citations, content } => {
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
            Inline::Ref(reference) => {
                self.output.push('[');
                self.output.push_str(reference);
                self.output.push_str("](#");
                self.output.push_str(reference);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    #[test]
    fn test_convert_paragraph() {
        let latex = "Hello world";
        let mut lexer = Lexer::new(latex);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let doc = parser.parse().unwrap();
        let mut converter = MarkdownConverter::new();
        let markdown = converter.convert(doc).unwrap();
        assert!(markdown.contains("Hello world"));
    }

    #[test]
    fn test_convert_section() {
        let latex = "\\section{Introduction}";
        let mut lexer = Lexer::new(latex);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let doc = parser.parse().unwrap();
        let mut converter = MarkdownConverter::new();
        let markdown = converter.convert(doc).unwrap();
        assert!(markdown.contains("## Introduction"));
    }

    #[test]
    fn test_convert_emphasis() {
        let latex = "\\textit{italic} and \\textbf{bold}";
        let mut lexer = Lexer::new(latex);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        let doc = parser.parse().unwrap();
        let mut converter = MarkdownConverter::new();
        let markdown = converter.convert(doc).unwrap();
        assert!(markdown.contains("*italic*"));
        assert!(markdown.contains("**bold**"));
    }
}
