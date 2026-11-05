use crate::error::Result;
use crate::parser::{Alignment, Block, Document, Inline, QuoteType};
use std::fmt::Write;

/// Table columns are padded to align, up to this many characters.
const MAX_PADDED_COLUMN_WIDTH: usize = 40;

pub struct MarkdownConverter {
    output: String,
    in_table: bool,
    frontmatter: bool,
    /// Nesting of emphasis/strong while rendering inlines (LaTeX toggles:
    /// \emph inside \emph is upright)
    emph_depth: usize,
    strong_depth: usize,
    /// Kind of the list block just emitted, so two adjacent lists of the
    /// same kind are kept apart instead of merging in Markdown
    last_list: Option<char>,
}

impl MarkdownConverter {
    pub fn new() -> Self {
        Self {
            output: String::new(),
            in_table: false,
            frontmatter: true,
            emph_depth: 0,
            strong_depth: 0,
            last_list: None,
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
        finalize_brackets(&escape_line_starts(converter.output.trim()))
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
                // Footnote marks (from \thanks) belong in the byline, not
                // in metadata; placeholders become their plain characters
                let value = strip_footnote_marks(&document.metadata[key]);
                let value = finalize_brackets(&value);
                writeln!(self.output, "{}: {}", key, yaml_scalar(&value)).unwrap();
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
                let text = document.footnotes[num].replace('\n', "\n    ");
                writeln!(self.output, "[^{}]: {}", num, text).unwrap();
            }
        }

        if let Some(bibliography) = &document.bibliography {
            if !self.output.ends_with("\n\n") {
                self.output.push('\n');
            }
            self.output.push_str(bibliography);
        }

        // Literal brackets not yet resolved by a paragraph-level pass
        Ok(finalize_brackets(&std::mem::take(&mut self.output)))
    }

    fn convert_block(&mut self, block: &Block) -> Result<()> {
        let list_kind = match block {
            Block::BulletList(_) => Some('-'),
            Block::OrderedList { .. } => Some('1'),
            Block::DescriptionList(_) => Some('-'),
            Block::Composite(_) | Block::Null => {
                return self.convert_block_inner(block);
            }
            _ => None,
        };
        if let (Some(kind), Some(last)) = (list_kind, self.last_list) {
            if kind == last && self.output.ends_with("\n\n") {
                // Two consecutive lists of one kind would merge into one
                self.output.push_str("<!-- -->\n\n");
            }
        }
        self.convert_block_inner(block)?;
        self.last_list = list_kind;
        Ok(())
    }

    fn convert_block_inner(&mut self, block: &Block) -> Result<()> {
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
                // Unconverted LaTeX is fenced so none of it is misread as
                // Markdown (lines starting with #, -, >, ...)
                self.convert_code_block(Some("latex"), content.trim_matches('\n'))?;
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

        // A heading is one line: line breaks in the title become spaces
        let heading = self.render_inlines_trimmed(title)?;
        let mut heading = heading
            .split('\n')
            .map(|line| line.trim_end_matches(' '))
            .collect::<Vec<_>>()
            .join(" ");
        if heading.is_empty() && label.is_none() {
            return Ok(());
        }
        // A trailing " #" would be read as a closing sequence
        if heading.ends_with('#') {
            let trimmed = heading.trim_end_matches('#');
            if trimmed.is_empty() || trimmed.ends_with(' ') {
                let hashes = heading.len() - trimmed.len();
                heading = format!("{}{}", trimmed, "\\#".repeat(hashes));
            }
        }

        for _ in 0..md_level {
            self.output.push('#');
        }
        self.output.push(' ');
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
            let text = finalize_brackets(&escape_line_starts(&text));
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

    /// Description lists render as bullet items led by the bold term:
    /// `- **Term**: description` (portable, unlike definition-list syntax).
    fn convert_description_list(&mut self, items: &[(Vec<Inline>, Vec<Block>)]) -> Result<()> {
        for (term, description) in items {
            let term_text = self.render_inlines_trimmed(term)?;
            let mut blocks: Vec<Block> = description.clone();
            if !term_text.is_empty() {
                let mut lead = vec![Inline::Strong(term.clone())];
                match blocks.first_mut() {
                    Some(Block::Paragraph(inlines)) => {
                        lead.push(Inline::Text(String::from(":")));
                        if !inlines.is_empty() {
                            lead.push(Inline::Space);
                        }
                        lead.append(inlines);
                        *inlines = lead;
                    }
                    _ => blocks.insert(0, Block::Paragraph(lead)),
                }
            }
            self.convert_list_item("- ", &blocks)?;
        }
        self.output.push('\n');
        Ok(())
    }

    fn convert_quote(&mut self, blocks: &[Block]) -> Result<()> {
        let saved_output = std::mem::take(&mut self.output);

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
        // The fence must be longer than any backtick run in the code
        let longest = longest_run(content, '`');
        let fence = "`".repeat((longest + 1).max(3));
        self.output.push_str(&fence);
        if let Some(lang) = language {
            let lang: String = lang
                .trim()
                .chars()
                .filter(|c| !c.is_whitespace() && *c != '`')
                .collect();
            self.output.push_str(&lang);
        }
        self.output.push('\n');
        let body = content.trim_end();
        let body = body.strip_prefix('\n').unwrap_or(body);
        self.output.push_str(body);
        self.output.push('\n');
        self.output.push_str(&fence);
        self.output.push_str("\n\n");
        Ok(())
    }

    fn convert_verbatim(&mut self, content: &str) -> Result<()> {
        self.convert_code_block(None, content)
    }

    fn convert_display_math(&mut self, content: &str) -> Result<()> {
        // A blank line would end the math block in Markdown renderers
        let body: Vec<&str> = content
            .trim()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        if body.is_empty() {
            return Ok(());
        }
        self.output.push_str("$$\n");
        self.output.push_str(&body.join("\n").replace("$$", "$ $"));
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

        // Pad for readability, but never to more than a modest width: one
        // long cell must not inflate every row of the table.
        for width in widths.iter_mut() {
            *width = (*width).min(MAX_PADDED_COLUMN_WIDTH);
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
            let alt = self.render_inlines_trimmed(cap)?;
            self.output.push_str(&escape_all_brackets(&alt.replace('\n', " ")));
        }
        self.output.push_str("](");
        self.output.push_str(&markdown_url(path));
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
        title: Option<&[Inline]>,
        content: &[Block],
    ) -> Result<()> {
        let title = match title {
            Some(inlines) => Some(self.render_inlines_trimmed(inlines)?.replace('\n', " ")),
            None => None,
        };
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
        
        if let Some(t) = title.filter(|t| !t.is_empty()) {
            self.output.push_str(" (");
            self.output.push_str(&t);
            self.output.push(')');
        }

        self.output.push_str("**");

        if let Some(label) = label {
            write!(self.output, " {{#{}}}", label).unwrap();
        }

        self.output.push_str("\n>\n");

        // Convert content as quoted
        let saved_output = std::mem::take(&mut self.output);

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

    /// Render `content` wrapped in a Markdown delimiter run (`*`, `**`,
    /// `~~`), falling back to the equivalent HTML tag where CommonMark's
    /// flanking rules would make the delimiters inert. Whitespace at the
    /// edges is moved outside; empty content renders nothing.
    fn convert_delimited(&mut self, content: &[Inline], delimiter: &str, tag: &str) -> Result<()> {
        let saved = std::mem::take(&mut self.output);
        let result = self.convert_inlines(content);
        let inner = std::mem::replace(&mut self.output, saved);
        result?;

        let leading_ws = inner.len() - inner.trim_start().len();
        let trailing_ws = inner.len() - inner.trim_end().len();
        let core = inner.trim();
        if core.is_empty() {
            if !inner.is_empty() && !self.output.ends_with(' ') && !self.output.is_empty() {
                self.output.push(' ');
            }
            return Ok(());
        }
        if leading_ws > 0 && !self.output.ends_with([' ', '\n']) && !self.output.is_empty() {
            self.output.push(' ');
        }

        let delim_char = delimiter.chars().next().unwrap();
        let prev = self.output.chars().next_back();
        let first = core.chars().next().unwrap();
        let last = core.chars().next_back().unwrap();
        let use_html = core.contains('\n')
            // adjacent delimiter runs (`*a***b**`) are ambiguous
            || prev == Some(delim_char)
            || first == delim_char
            || last == delim_char
            // `x*(y)*`: an opening run between a letter and punctuation is
            // not left-flanking
            || (prev.is_some_and(|c| c.is_alphanumeric()) && is_md_punctuation(first))
            || (is_md_punctuation(last) && prev.is_some_and(|c| c.is_alphanumeric()));
        if use_html {
            write!(self.output, "<{}>{}</{}>", tag, core, tag).unwrap();
        } else {
            self.output.push_str(delimiter);
            self.output.push_str(core);
            self.output.push_str(delimiter);
        }
        if trailing_ws > 0 {
            self.output.push(' ');
        }
        Ok(())
    }

    fn render_inlines_trimmed(&mut self, inlines: &[Inline]) -> Result<String> {
        let saved_output = std::mem::take(&mut self.output);
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
                if self.emph_depth > 0 {
                    // \emph inside \emph switches back to upright
                    self.emph_depth -= 1;
                    let result = self.convert_inlines(content);
                    self.emph_depth += 1;
                    result?;
                } else {
                    self.emph_depth += 1;
                    let result = self.convert_delimited(content, "*", "em");
                    self.emph_depth -= 1;
                    result?;
                }
            }
            Inline::Strong(content) => {
                if self.strong_depth > 0 {
                    self.convert_inlines(content)?;
                } else {
                    self.strong_depth += 1;
                    let result = self.convert_delimited(content, "**", "strong");
                    self.strong_depth -= 1;
                    result?;
                }
            }
            Inline::Strikeout(content) => {
                self.convert_delimited(content, "~~", "del")?;
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
                let code = code.replace('\n', " ");
                if code.is_empty() {
                    // `` is not a code span
                } else {
                    // The fence must be longer than any backtick run inside;
                    // pad when the code starts/ends with a backtick or space
                    let fence = "`".repeat(longest_run(&code, '`') + 1);
                    let pad = code.starts_with('`')
                        || code.ends_with('`')
                        || (code.starts_with(' ') && code.ends_with(' ') && !code.trim().is_empty());
                    self.output.push_str(&fence);
                    if pad {
                        self.output.push(' ');
                    }
                    self.output.push_str(&code);
                    if pad {
                        self.output.push(' ');
                    }
                    self.output.push_str(&fence);
                }
            }
            Inline::InlineMath(math) if math.trim().is_empty() => {
                // `$$` would open display math in Markdown: emit nothing
            }
            Inline::InlineMath(math) => {
                let math = if self.in_table {
                    math.replace('|', "\\|")
                } else {
                    math.clone()
                };
                self.output.push('$');
                self.output.push_str(&math.trim().replace('\n', " "));
                self.output.push('$');
            }
            Inline::Link { text, url, title } => {
                let rendered = self.render_inlines_trimmed(text)?;
                let plain_url = url.trim();
                // \url{https://...}: an autolink is the cleanest form
                if title.is_none()
                    && is_autolinkable(plain_url)
                    && text.len() == 1
                    && matches!(&text[0], Inline::Text(t) if t.trim() == plain_url)
                {
                    self.output.push('<');
                    self.output.push_str(plain_url);
                    self.output.push('>');
                    return Ok(());
                }
                let rendered = if rendered.is_empty() {
                    escape_markdown_text(plain_url, false)
                } else {
                    rendered
                };
                self.output.push('[');
                self.output.push_str(&escape_all_brackets(&rendered.replace('\n', " ")));
                self.output.push_str("](");
                self.output.push_str(&markdown_url(plain_url));
                if let Some(t) = title {
                    self.output.push_str(" \"");
                    self.output.push_str(t);
                    self.output.push('"');
                }
                self.output.push(')');
            }
            Inline::Image { alt, url, title } => {
                let rendered = self.render_inlines_trimmed(alt)?;
                self.output.push_str("![");
                self.output.push_str(&escape_all_brackets(&rendered.replace('\n', " ")));
                self.output.push_str("](");
                self.output.push_str(&markdown_url(url.trim()));
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
    let mut out = String::with_capacity(text.len() + 8);
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' | '*' | '_' | '`' | '$' | '<' | '~' => {
                out.push('\\');
                out.push(ch);
            }
            '|' if in_table => {
                out.push('\\');
                out.push(ch);
            }
            // `&name;` / `&#123;` would be decoded as an HTML entity; the
            // following text may be a separate token, so decide later
            '&' => out.push(AMPERSAND),
            // Brackets only form links in specific patterns; decided once
            // the whole paragraph is known (see finalize_brackets)
            '[' => out.push(OPEN_BRACKET),
            ']' => out.push(CLOSE_BRACKET),
            _ => out.push(ch),
        }
    }
    out
}

/// Placeholders for literal brackets from document text (Unicode private
/// use characters, never present in real output).
const OPEN_BRACKET: char = '\u{F8F0}';
const CLOSE_BRACKET: char = '\u{F8F1}';
const AMPERSAND: char = '\u{F8F2}';

/// Resolve literal-bracket placeholders: plain brackets, unless the text
/// contains a sequence Markdown would read as link syntax (`](`, `][`,
/// `]:`), in which case every literal bracket is escaped.
fn finalize_brackets(text: &str) -> String {
    let text = &finalize_ampersands(text);
    if !text.contains([OPEN_BRACKET, CLOSE_BRACKET]) {
        return text.to_string();
    }
    let risky = text
        .char_indices()
        .any(|(i, c)| c == CLOSE_BRACKET && text[i + c.len_utf8()..].starts_with(['(', '[', ':', OPEN_BRACKET]));
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            OPEN_BRACKET if risky => out.push_str("\\["),
            CLOSE_BRACKET if risky => out.push_str("\\]"),
            OPEN_BRACKET => out.push('['),
            CLOSE_BRACKET => out.push(']'),
            _ => out.push(c),
        }
    }
    out
}

/// Resolve literal-ampersand placeholders: escaped only where they would
/// start an HTML entity reference (`&amp;`, `&#35;`, `&#x41;`).
fn finalize_ampersands(text: &str) -> String {
    if !text.contains(AMPERSAND) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    for (i, c) in text.char_indices() {
        if c != AMPERSAND {
            out.push(c);
            continue;
        }
        let rest = &text[i + c.len_utf8()..];
        let name_len = rest
            .char_indices()
            .take_while(|(_, ch)| ch.is_ascii_alphanumeric() || *ch == '#')
            .count();
        if name_len > 0 && rest[name_len..].starts_with(';') {
            out.push_str("\\&");
        } else {
            out.push('&');
        }
    }
    out
}

/// Escape every literal bracket (inside link text and image alt text).
fn escape_all_brackets(text: &str) -> String {
    finalize_ampersands(text)
        .replace(OPEN_BRACKET, "\\[")
        .replace(CLOSE_BRACKET, "\\]")
}

/// Escape characters at the start of each line that Markdown would read as
/// block syntax: headings, quotes, list markers, setext underlines.
fn escape_line_starts(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 4);
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let indent = line.len() - line.trim_start_matches(' ').len();
        let (lead, rest) = line.split_at(indent);
        out.push_str(lead);
        out.push_str(&escape_block_start(rest));
    }
    out
}

fn escape_block_start(line: &str) -> String {
    let bytes = line.as_bytes();
    let Some(&first) = bytes.first() else {
        return String::new();
    };
    let followed_by_space_or_end = |n: usize| bytes.get(n).is_none_or(|b| *b == b' ' || *b == b'\t');
    match first {
        b'#' => {
            let hashes = bytes.iter().take_while(|&&b| b == b'#').count();
            if hashes <= 6 && followed_by_space_or_end(hashes) {
                return format!("\\{}", line);
            }
        }
        b'>' => return format!("\\{}", line),
        b'-' | b'+' | b'*' if followed_by_space_or_end(1) => return format!("\\{}", line),
        b'=' | b'-' if line.trim_end().bytes().all(|b| b == first) => return format!("\\{}", line),
        b'0'..=b'9' => {
            let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
            if digits <= 9 && matches!(bytes.get(digits), Some(b'.') | Some(b')')) && followed_by_space_or_end(digits + 1) {
                return format!("{}\\{}", &line[..digits], &line[digits..]);
            }
        }
        _ => {}
    }
    line.to_string()
}

/// ASCII punctuation as defined by CommonMark (for flanking rules).
fn is_md_punctuation(c: char) -> bool {
    c.is_ascii_punctuation() || matches!(c, '\u{201C}' | '\u{201D}' | '\u{2018}' | '\u{2019}' | '\u{2013}' | '\u{2014}' | '\u{2026}')
}

/// Length of the longest run of `ch` in `text`.
fn longest_run(text: &str, ch: char) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for c in text.chars() {
        if c == ch {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

/// Is this URL safe to emit as an autolink `<url>`?
fn is_autolinkable(url: &str) -> bool {
    let has_scheme = url
        .split_once(':')
        .is_some_and(|(scheme, rest)| {
            (2..=32).contains(&scheme.len())
                && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
                && scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && !rest.is_empty()
        });
    has_scheme && !url.contains(|c: char| c.is_whitespace() || c == '<' || c == '>' || c.is_control())
}

/// Make a URL safe as a Markdown link destination.
fn markdown_url(url: &str) -> String {
    let needs_brackets = url.contains(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == '<' || c == '>')
        || url.is_empty();
    let url = url.replace('\n', "");
    if needs_brackets {
        format!("<{}>", url.replace('<', "%3C").replace('>', "%3E"))
    } else {
        url
    }
}

/// Remove `[^n]` footnote references from a metadata value.
fn strip_footnote_marks(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("[^") {
        let after = &rest[start + 2..];
        let digits = after.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 && after[digits..].starts_with(']') {
            out.push_str(&rest[..start]);
            rest = &after[digits + 1..];
        } else {
            out.push_str(&rest[..start + 2]);
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// Quote a YAML scalar when needed.
fn yaml_scalar(value: &str) -> String {
    // Hard line breaks in a title/author become spaces in metadata
    let value: String = value
        .split('\n')
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join(" ");
    let lower = value.to_ascii_lowercase();
    let plain_safe = !value.is_empty()
        && value.chars().all(|c| !c.is_control())
        && !value.starts_with(|c: char| {
            c.is_whitespace() || "-?:,[]{}#&*!|>'\"%@`".contains(c)
        })
        && !value.ends_with(|c: char| c.is_whitespace() || c == ':')
        && !value.contains(": ")
        && !value.contains(" #")
        && !matches!(
            lower.as_str(),
            "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "~" | "y" | "n"
        )
        // numbers would change type (keep dates like 2020 as text too)
        && value.parse::<f64>().is_err();
    if plain_safe {
        return value;
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\x{:02X}", c as u32 & 0xFF)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
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
