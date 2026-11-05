//! Tables: table floats (captions, notes, several tabulars, images of
//! tables) and tabular-like environments (nested tabulars, block content in
//! p-columns, longtable head/foot segments, longtable captions).

use super::{trim_inline_whitespace, Alignment, Block, Inline, Parser};
use crate::error::Result;
use crate::lexer::Token;
use crate::state::LabelType;

/// Tabular-like environments.
pub(super) fn is_tabular_env(name: &str) -> bool {
    matches!(
        name,
        "tabular" | "tabular*" | "tabularx" | "tabulary" | "longtable" | "longtable*" | "array"
            | "tabu" | "longtabu" | "supertabular" | "supertabular*" | "xtabular" | "mpsupertabular"
            | "tblr" | "longtblr" | "talltblr" | "NiceTabular" | "NiceTabular*" | "NiceArray"
            | "tabularew" | "ltablex"
    )
}

/// A parsed tabular plus the caption/label a longtable may carry.
pub(super) struct ParsedTabular {
    pub alignments: Vec<Alignment>,
    pub headers: Vec<Vec<Block>>,
    pub rows: Vec<Vec<Vec<Block>>>,
    pub caption: Option<(Vec<Inline>, bool)>,
    pub label: Option<String>,
}

impl Parser {
    /// `\begin{table}...\end{table}` and friends.
    pub(super) fn parse_table_float(&mut self, env_name: &str) -> Result<Option<Block>> {
        self.skip_whitespace_and_comments();
        if matches!(self.current_token(), Token::LeftBracket) {
            let _ = self.collect_bracketed_tokens();
        }
        let mut caption: Option<(Vec<Inline>, bool)> = None;
        let mut label = None;
        let mut tables: Vec<ParsedTabular> = Vec::new();
        let mut extra: Vec<Block> = Vec::new();
        let mut text: Vec<Inline> = Vec::new();

        while !self.is_at_end() {
            match self.current_token().clone() {
                Token::EndEnvironment(env) if env == env_name => {
                    self.advance();
                    break;
                }
                Token::Label(l) => {
                    self.advance();
                    label = Some(l);
                }
                Token::Command(cmd) if cmd == "caption" || cmd == "captionof" => {
                    self.advance();
                    caption = Some(self.parse_float_caption(cmd == "captionof")?);
                    if let Some(l) = self.pending_caption_label.take() {
                        label = Some(l);
                    }
                }
                Token::Command(cmd) if cmd == "includegraphics" => {
                    // A table typeset as an image
                    self.advance();
                    self.consume_text_star();
                    while matches!(self.current_token(), Token::LeftBracket) {
                        let _ = self.collect_bracketed_tokens();
                    }
                    let path = self.parse_braced_text()?.trim().to_string();
                    if !path.is_empty() {
                        extra.push(Block::Figure {
                            caption: None,
                            path,
                            label: None,
                        });
                    }
                }
                Token::BeginEnvironment(env) if is_tabular_env(&env) => {
                    self.advance();
                    let parsed = self.parse_tabular_body(&env)?;
                    tables.push(parsed);
                }
                Token::BeginEnvironment(env) if matches!(env.as_str(), "tablenotes" | "TableNotes" | "tnotes") => {
                    if let Some(block) = self.parse_block()? {
                        extra.push(block);
                    }
                }
                Token::BeginEnvironment(env)
                    if matches!(env.as_str(), "itemize" | "enumerate" | "description" | "minipage" | "subtable") =>
                {
                    if let Some(block) = self.parse_block()? {
                        extra.push(block);
                    }
                }
                // center, threeparttable, adjustbox, small, ...: transparent
                Token::BeginEnvironment(_) | Token::EndEnvironment(_) => self.advance(),
                Token::MathEnv { .. } | Token::DisplayMath(_) | Token::VerbatimEnv { .. } => {
                    if let Some(block) = self.parse_block()? {
                        extra.push(block);
                    }
                }
                Token::ParBreak => {
                    self.advance();
                    if !trim_inline_whitespace(text.clone()).is_empty() {
                        extra.push(Block::Paragraph(trim_inline_whitespace(std::mem::take(&mut text))));
                    }
                    text.clear();
                }
                Token::LeftBrace | Token::RightBrace | Token::Comment(_) => self.advance(),
                Token::Command(cmd)
                    if matches!(
                        cmd.as_str(),
                        "resizebox" | "scalebox" | "adjustbox" | "centering" | "small" | "footnotesize"
                            | "scriptsize" | "tiny" | "normalsize" | "large" | "rowcolors"
                            | "setlength" | "renewcommand" | "arraystretch" | "def"
                    ) =>
                {
                    // Wrappers/size switches: their arguments hold the tabular
                    self.advance();
                    match cmd.as_str() {
                        "resizebox" => {
                            for _ in 0..2 {
                                self.skip_whitespace_only();
                                let _ = self.collect_braced_tokens();
                            }
                        }
                        "scalebox" => {
                            self.skip_whitespace_only();
                            let _ = self.collect_braced_tokens();
                            if matches!(self.current_token(), Token::LeftBracket) {
                                let _ = self.collect_bracketed_tokens();
                            }
                        }
                        "adjustbox" => {
                            self.skip_whitespace_only();
                            let _ = self.collect_braced_tokens();
                        }
                        "rowcolors" => self.consume_trailing_groups(3)?,
                        "setlength" | "renewcommand" | "def" => {
                            self.push_front(vec![Token::Command(cmd.clone())]);
                            let _ = self.parse_inline()?;
                        }
                        _ => {}
                    }
                }
                _ => {
                    if let Some(inline) = self.parse_inline()? {
                        text.push(inline);
                    }
                }
            }
        }
        let leftover = trim_inline_whitespace(text);
        if !leftover.is_empty() {
            extra.push(Block::Paragraph(leftover));
        }

        // A longtable inside the float may carry the caption itself
        if caption.is_none() {
            if let Some(t) = tables.iter_mut().find(|t| t.caption.is_some()) {
                caption = t.caption.take();
                if label.is_none() {
                    label = t.label.take();
                }
            }
        }
        Ok(self.assemble_table(tables, caption, label, extra))
    }

    /// A tabular environment on its own (not inside a table float).
    pub(super) fn parse_standalone_tabular(&mut self, env_name: &str) -> Result<Option<Block>> {
        let mut parsed = self.parse_tabular_body(env_name)?;
        let caption = parsed.caption.take();
        let label = parsed.label.take();
        Ok(self.assemble_table(vec![parsed], caption, label, Vec::new()))
    }

    /// Caption text after `\caption` (with optional `*` and `[short]`).
    /// Returns (inlines, numbered). A `\label` inside is stashed in
    /// `pending_caption_label`.
    fn parse_float_caption(&mut self, captionof: bool) -> Result<(Vec<Inline>, bool)> {
        let unnumbered = self.consume_text_star();
        if captionof {
            self.skip_whitespace_only();
            let _ = self.parse_braced_text()?;
        }
        self.skip_whitespace_only();
        if matches!(self.current_token(), Token::LeftBracket) {
            let _ = self.collect_bracketed_tokens();
        }
        let tokens = self.collect_braced_tokens();
        let (inlines, label) = self.parse_inline_fragment_tokens(tokens)?;
        self.pending_caption_label = label;
        Ok((trim_inline_whitespace(inlines), !unnumbered))
    }

    fn assemble_table(
        &mut self,
        tables: Vec<ParsedTabular>,
        caption: Option<(Vec<Inline>, bool)>,
        label: Option<String>,
        extra: Vec<Block>,
    ) -> Option<Block> {
        let number = match &caption {
            Some((_, true)) => Some(self.state.next_counter("table")),
            _ => None,
        };
        if let Some(label) = &label {
            let title = caption
                .as_ref()
                .map(|(c, _)| self.inlines_to_plain_text(c))
                .filter(|t| !t.is_empty());
            let n = number
                .clone()
                .unwrap_or_else(|| self.state.current_section_number.clone());
            self.state.add_label_with_number(label.clone(), LabelType::Table, n, title);
        }
        let caption_inlines = caption.map(|(text, _)| {
            let mut line = Vec::new();
            if let Some(n) = &number {
                line.push(Inline::Strong(vec![Inline::Text(format!("Table {}:", n))]));
                if !text.is_empty() {
                    line.push(Inline::Space);
                }
            }
            line.extend(text);
            line
        });

        let mut blocks = Vec::new();
        let mut caption_inlines = caption_inlines;
        for table in tables {
            if table.headers.is_empty() && table.rows.is_empty() {
                continue;
            }
            blocks.push(Block::Table {
                caption: caption_inlines.take(),
                alignments: table.alignments,
                headers: table.headers,
                rows: table.rows,
            });
        }
        if let Some(caption) = caption_inlines.filter(|c| !c.is_empty()) {
            // Caption without a (non-empty) tabular
            blocks.insert(0, Block::Paragraph(caption));
        }
        blocks.extend(extra);
        match blocks.len() {
            0 => None,
            1 => blocks.pop(),
            _ => Some(Block::Composite(blocks)),
        }
    }

    /// Parse a tabular-like environment body (after `\begin{env}`).
    pub(super) fn parse_tabular_body(&mut self, env_name: &str) -> Result<ParsedTabular> {
        // [pos], then {width} for tabular*/tabularx/tabulary, then {colspec}
        self.skip_whitespace_and_comments();
        if matches!(self.current_token(), Token::LeftBracket) {
            let _ = self.collect_bracketed_tokens();
        }
        if matches!(env_name, "tabular*" | "tabularx" | "tabulary" | "xtabular*" | "supertabular*" | "NiceTabular*" | "ltablex") {
            self.skip_whitespace_and_comments();
            let _ = self.parse_braced_text()?;
        }
        if matches!(env_name, "tabu" | "longtabu") {
            // \begin{tabu} to \linewidth {spec}
            while matches!(self.current_token(), Token::Text(_) | Token::Whitespace(_) | Token::Command(_)) {
                self.advance();
            }
        }
        self.skip_whitespace_and_comments();
        let alignments = if matches!(self.current_token(), Token::LeftBrace) {
            let spec = self.parse_braced_text()?;
            self.parse_column_spec(&spec)
        } else {
            vec![Alignment::AlignDefault]
        };
        if matches!(self.current_token(), Token::LeftBracket) {
            // tblr/NiceTabular trailing options
            let _ = self.collect_bracketed_tokens();
        }

        let mut rows: Vec<Vec<Vec<Block>>> = Vec::new();
        let mut current_row: Vec<Vec<Block>> = Vec::new();
        let mut cell: Vec<Inline> = Vec::new();
        let mut pending_span_cells: usize = 0;
        let mut caption = None;
        let mut label = None;
        // longtable head/foot segments
        let mut head_rows: Option<Vec<Vec<Vec<Block>>>> = None;
        let mut segment_start = 0usize;

        fn finish_cell(cell: &mut Vec<Inline>, row: &mut Vec<Vec<Block>>, pending: &mut usize) {
            let trimmed = trim_inline_whitespace(std::mem::take(cell));
            if trimmed.is_empty() {
                row.push(Vec::new());
            } else {
                row.push(vec![Block::Paragraph(trimmed)]);
            }
            for _ in 0..*pending {
                row.push(Vec::new());
            }
            *pending = 0;
        }

        while !self.is_at_end() {
            match self.current_token().clone() {
                Token::EndEnvironment(env) if env == env_name => {
                    self.advance();
                    break;
                }
                // A mismatched \end (malformed input) also ends the table
                Token::EndEnvironment(env)
                    if !self.state.environments.contains_key(&env) && !env.is_empty() =>
                {
                    break;
                }
                Token::Ampersand => {
                    self.advance();
                    finish_cell(&mut cell, &mut current_row, &mut pending_span_cells);
                }
                Token::Backslash => {
                    self.advance();
                    self.consume_text_star();
                    if matches!(self.current_token(), Token::LeftBracket) {
                        let _ = self.collect_bracketed_tokens();
                    }
                    finish_cell(&mut cell, &mut current_row, &mut pending_span_cells);
                    rows.push(std::mem::take(&mut current_row));
                }
                Token::Command(cmd) if matches!(cmd.as_str(), "tabularnewline" | "cr" | "crcr" | "AND") => {
                    self.advance();
                    finish_cell(&mut cell, &mut current_row, &mut pending_span_cells);
                    rows.push(std::mem::take(&mut current_row));
                }
                Token::Command(cmd) if matches!(cmd.as_str(), "endfirsthead" | "endhead" | "endfoot" | "endlastfoot") => {
                    self.advance();
                    if !cell.is_empty() || !current_row.is_empty() {
                        finish_cell(&mut cell, &mut current_row, &mut pending_span_cells);
                        rows.push(std::mem::take(&mut current_row));
                    }
                    match cmd.as_str() {
                        "endfirsthead" => {
                            head_rows = Some(rows.drain(..).collect());
                        }
                        "endhead" => {
                            if head_rows.is_some() {
                                // continuation head: repeated on later pages
                                rows.truncate(segment_start.min(rows.len()));
                            } else {
                                head_rows = Some(rows.drain(..).collect());
                            }
                        }
                        _ => {
                            // footers ("continued on next page")
                            rows.truncate(segment_start.min(rows.len()));
                        }
                    }
                    segment_start = rows.len();
                }
                Token::Command(cmd) if super::is_table_rule_command(&cmd) => {
                    self.advance();
                    self.consume_table_rule_args(&cmd)?;
                }
                Token::Command(cmd) if cmd == "caption" => {
                    // longtable caption row
                    self.advance();
                    caption = Some(self.parse_float_caption(false)?);
                    if let Some(l) = self.pending_caption_label.take() {
                        label = Some(l);
                    }
                }
                Token::Command(cmd) if cmd == "multicolumn" => {
                    self.advance();
                    let span = self.parse_braced_text()?.trim().parse::<usize>().unwrap_or(1).min(super::MAX_TABLE_COLUMNS);
                    self.skip_whitespace_only();
                    let _alignment = self.parse_braced_text()?;
                    self.skip_whitespace_only();
                    let tokens = self.collect_braced_tokens();
                    let inlines = self.parse_cell_tokens(tokens)?;
                    cell.extend(inlines);
                    pending_span_cells = pending_span_cells.max(span.saturating_sub(1));
                }
                Token::Command(cmd) if matches!(cmd.as_str(), "multirow" | "multirowcell") => {
                    self.advance();
                    self.skip_whitespace_only();
                    if matches!(self.current_token(), Token::LeftBracket) {
                        let _ = self.collect_bracketed_tokens();
                    }
                    let _rows = self.parse_braced_text()?;
                    self.skip_whitespace_only();
                    if matches!(self.current_token(), Token::LeftBracket) {
                        let _ = self.collect_bracketed_tokens();
                    }
                    if cmd == "multirow" {
                        self.skip_whitespace_only();
                        let _width = self.parse_braced_text()?;
                        self.skip_whitespace_only();
                        if matches!(self.current_token(), Token::LeftBracket) {
                            let _ = self.collect_bracketed_tokens();
                        }
                    }
                    self.skip_whitespace_only();
                    let tokens = self.collect_braced_tokens();
                    let inlines = self.parse_cell_tokens(tokens)?;
                    cell.extend(inlines);
                }
                Token::BeginEnvironment(env) if is_tabular_env(&env) => {
                    // Nested table: flattened into the cell
                    self.advance();
                    let nested = self.parse_tabular_body(&env)?;
                    cell.extend(flatten_table(nested));
                }
                Token::BeginEnvironment(_) | Token::MathEnv { .. } | Token::DisplayMath(_) | Token::VerbatimEnv { .. } => {
                    // Block content in a p-column cell (lists, minipages, math)
                    if let Some(block) = self.parse_block()? {
                        cell.extend(block_to_cell_inlines(block));
                    }
                }
                Token::ParBreak => {
                    self.advance();
                    if !cell.is_empty() {
                        cell.push(Inline::LineBreak);
                    }
                }
                Token::Label(l) => {
                    self.advance();
                    label = Some(l);
                }
                Token::Comment(_) => self.advance(),
                _ => {
                    if let Some(inline) = self.parse_inline()? {
                        cell.push(inline);
                    }
                }
            }
        }

        if !trim_inline_whitespace(cell.clone()).is_empty() || !current_row.is_empty() || pending_span_cells > 0 {
            finish_cell(&mut cell, &mut current_row, &mut pending_span_cells);
            rows.push(current_row);
        }

        let mut all_rows: Vec<Vec<Vec<Block>>> = Vec::new();
        if let Some(head) = head_rows {
            all_rows.extend(head);
        }
        all_rows.extend(rows);
        // Drop rows that are entirely empty (trailing \\, rule-only rows)
        all_rows.retain(|row| row.iter().any(|cell| !cell.is_empty()));

        let headers = if all_rows.is_empty() {
            Vec::new()
        } else {
            all_rows.remove(0)
        };
        Ok(ParsedTabular {
            alignments,
            headers,
            rows: all_rows,
            caption,
            label,
        })
    }

    /// Parse a token list as cell content.
    fn parse_cell_tokens(&mut self, tokens: Vec<Token>) -> Result<Vec<Inline>> {
        let (inlines, _) = self.parse_inline_fragment_tokens(tokens)?;
        Ok(inlines)
    }
}

/// A nested table as cell inlines: cells separated by spaces, rows by
/// line breaks.
fn flatten_table(table: ParsedTabular) -> Vec<Inline> {
    let mut out = Vec::new();
    let rows = std::iter::once(table.headers).chain(table.rows);
    for (ri, row) in rows.enumerate() {
        if row.iter().all(|c| c.is_empty()) {
            continue;
        }
        if ri > 0 && !out.is_empty() {
            out.push(Inline::LineBreak);
        }
        let mut first = true;
        for cell in row {
            let inlines: Vec<Inline> = cell.into_iter().flat_map(block_to_cell_inlines).collect();
            if inlines.is_empty() {
                continue;
            }
            if !first {
                out.push(Inline::Space);
            }
            first = false;
            out.extend(inlines);
        }
    }
    out
}

/// Block content flattened for a single-line table cell.
fn block_to_cell_inlines(block: Block) -> Vec<Inline> {
    match block {
        Block::Paragraph(inlines) => inlines,
        Block::DisplayMath(math) => vec![Inline::InlineMath(math)],
        Block::CodeBlock { content, .. } | Block::Verbatim(content) => {
            vec![Inline::Code(content.trim().replace('\n', " "))]
        }
        Block::BulletList(items) | Block::OrderedList { items, .. } => {
            let mut out = Vec::new();
            for (i, item) in items.into_iter().enumerate() {
                if i > 0 {
                    out.push(Inline::LineBreak);
                }
                out.push(Inline::Text(String::from("\u{2022} ")));
                for block in item {
                    out.extend(block_to_cell_inlines(block));
                }
            }
            out
        }
        Block::Composite(blocks) | Block::Quote(blocks) => {
            let mut out = Vec::new();
            for block in blocks {
                if !out.is_empty() {
                    out.push(Inline::LineBreak);
                }
                out.extend(block_to_cell_inlines(block));
            }
            out
        }
        Block::Figure { path, caption, .. } => vec![Inline::Image {
            alt: caption.unwrap_or_default(),
            url: path,
            title: None,
        }],
        Block::Table { headers, rows, alignments, caption } => flatten_table(ParsedTabular {
            alignments,
            headers,
            rows,
            caption: caption.map(|c| (c, false)),
            label: None,
        }),
        Block::Section { title, .. } => vec![Inline::Strong(title)],
        _ => Vec::new(),
    }
}
