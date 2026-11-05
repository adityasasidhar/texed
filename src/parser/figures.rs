//! Figure environments: images, sub-figures (subcaption's `subfigure`,
//! subfig's `\subfloat`, `\subcaptionbox`), minipages carrying their own
//! captions, and embedded content (TikZ, tables) inside the float.

use super::{Block, Inline, Parser};
use crate::error::Result;
use crate::lexer::Token;
use crate::state::{alpha_number, LabelType};

/// One captioned unit found inside a figure body.
#[derive(Default)]
struct FigurePart {
    images: Vec<String>,
    caption: Option<Vec<Inline>>,
    label: Option<String>,
    /// Non-image content (TikZ code, tables, text) in source order
    blocks: Vec<Block>,
    /// Sub-figures (each with its own caption/label)
    subs: Vec<FigurePart>,
    /// Minipages with their own \caption: separate numbered figures
    separate: Vec<FigurePart>,
}

impl Parser {
    pub(super) fn parse_figure_float(&mut self, env_name: &str) -> Result<Option<Block>> {
        // [placement]; wrapfigure also takes {placement}{width}
        self.skip_whitespace_and_comments();
        if matches!(self.current_token(), Token::LeftBracket) {
            let _ = self.collect_bracketed_tokens();
        }
        if env_name.starts_with("wrap") {
            for _ in 0..2 {
                self.skip_whitespace_and_comments();
                if matches!(self.current_token(), Token::LeftBrace) {
                    let _ = self.collect_braced_tokens();
                }
                if matches!(self.current_token(), Token::LeftBracket) {
                    let _ = self.collect_bracketed_tokens();
                }
            }
        }

        let mut part = FigurePart::default();
        self.scan_figure_body(Some(env_name), &mut part, 0)?;
        Ok(self.assemble_figure(part))
    }

    /// Scan tokens until `\end{end_env}` (or end of the token stream when
    /// `end_env` is None), collecting figure parts.
    fn scan_figure_body(&mut self, end_env: Option<&str>, part: &mut FigurePart, depth: usize) -> Result<()> {
        while !self.is_at_end() {
            let token = self.current_token().clone();
            match token {
                Token::EndEnvironment(env) if Some(env.as_str()) == end_env => {
                    self.advance();
                    return Ok(());
                }
                Token::Label(label) => {
                    self.advance();
                    part.label = Some(label);
                }
                Token::Command(cmd) => {
                    match cmd.as_str() {
                        "includegraphics" | "includesvg" | "includepdf" | "epsfig" | "epsfbox"
                        | "psfig" | "epsffile" => {
                            self.advance();
                            self.consume_text_star();
                            self.skip_whitespace_only();
                            let mut options = String::new();
                            while matches!(self.current_token(), Token::LeftBracket) {
                                options = self.parse_bracketed_text()?;
                            }
                            let mut path = self.parse_braced_text()?.trim().to_string();
                            // \epsfig{file=x.eps,width=...}, \psfig{figure=...}
                            if cmd == "epsfig" || cmd == "psfig" {
                                path = path
                                    .split(',')
                                    .find_map(|kv| {
                                        let (k, v) = kv.split_once('=')?;
                                        matches!(k.trim(), "file" | "figure").then(|| v.trim().to_string())
                                    })
                                    .unwrap_or(path);
                            }
                            let _ = options;
                            if !path.is_empty() {
                                part.images.push(path);
                            }
                        }
                        "caption" | "subcaption" | "captionof" => {
                            self.advance();
                            self.consume_text_star();
                            if cmd == "captionof" {
                                self.skip_whitespace_only();
                                let _ = self.parse_braced_text()?;
                            }
                            self.skip_whitespace_only();
                            if matches!(self.current_token(), Token::LeftBracket) {
                                let _ = self.collect_bracketed_tokens();
                            }
                            let tokens = self.collect_braced_tokens();
                            let (inlines, caption_label) = self.parse_inline_fragment_tokens(tokens)?;
                            part.caption = Some(super::trim_inline_whitespace(inlines));
                            if caption_label.is_some() {
                                part.label = caption_label;
                            }
                        }
                        "subfloat" | "subfigure" | "subtable" if depth < 4 => {
                            // \subfloat[caption]{content}
                            self.advance();
                            self.skip_whitespace_only();
                            let mut sub = FigurePart::default();
                            if matches!(self.current_token(), Token::LeftBracket) {
                                let tokens = self.collect_bracketed_tokens();
                                let (inlines, label) = self.parse_inline_fragment_tokens(tokens)?;
                                sub.caption = Some(super::trim_inline_whitespace(inlines)).filter(|c| !c.is_empty());
                                sub.label = label;
                            }
                            if matches!(self.current_token(), Token::LeftBracket) {
                                let _ = self.collect_bracketed_tokens();
                            }
                            self.skip_whitespace_only();
                            let content = self.collect_braced_tokens();
                            self.scan_figure_tokens(content, &mut sub, depth + 1)?;
                            part.subs.push(sub);
                        }
                        "subcaptionbox" if depth < 4 => {
                            // \subcaptionbox[list entry]{caption}[width]{content}
                            self.advance();
                            self.consume_text_star();
                            self.skip_whitespace_only();
                            if matches!(self.current_token(), Token::LeftBracket) {
                                let _ = self.collect_bracketed_tokens();
                            }
                            let mut sub = FigurePart::default();
                            let tokens = self.collect_braced_tokens();
                            let (inlines, label) = self.parse_inline_fragment_tokens(tokens)?;
                            sub.caption = Some(super::trim_inline_whitespace(inlines));
                            sub.label = label;
                            self.skip_whitespace_only();
                            while matches!(self.current_token(), Token::LeftBracket) {
                                let _ = self.collect_bracketed_tokens();
                            }
                            let content = self.collect_braced_tokens();
                            self.scan_figure_tokens(content, &mut sub, depth + 1)?;
                            part.subs.push(sub);
                        }
                        _ => self.advance(),
                    }
                }
                Token::BeginEnvironment(env) => {
                    match env.as_str() {
                        "subfigure" | "subtable" | "minipage" if depth < 4 => {
                            self.advance();
                            // [pos][height][inner-pos]{width}
                            self.skip_whitespace_and_comments();
                            while matches!(self.current_token(), Token::LeftBracket) {
                                let _ = self.collect_bracketed_tokens();
                                self.skip_whitespace_and_comments();
                            }
                            if matches!(self.current_token(), Token::LeftBrace) {
                                let _ = self.collect_braced_tokens();
                            }
                            let mut sub = FigurePart::default();
                            self.scan_figure_body(Some(&env), &mut sub, depth + 1)?;
                            if env == "minipage" {
                                if sub.caption.is_some() && depth == 0 {
                                    part.separate.push(sub);
                                } else {
                                    merge_into(part, sub);
                                }
                            } else {
                                part.subs.push(sub);
                            }
                        }
                        "tikzpicture" | "tabular" | "tabular*" | "tabularx" | "tabulary" | "longtable"
                        | "pgfpicture" | "picture" | "axis" | "tikzcd" | "forest" | "algorithmic"
                        | "lstlisting" | "verbatim" | "array" => {
                            if let Some(block) = self.parse_block()? {
                                part.blocks.push(block);
                            }
                        }
                        // center, flushleft, adjustbox, ...: transparent
                        _ => self.advance(),
                    }
                }
                Token::VerbatimEnv { .. } | Token::MathEnv { .. } | Token::DisplayMath(_) => {
                    if let Some(block) = self.parse_block()? {
                        part.blocks.push(block);
                    }
                }
                _ => self.advance(),
            }
        }
        Ok(())
    }

    /// Scan a token list (an argument's content) as figure body.
    fn scan_figure_tokens(&mut self, tokens: Vec<Token>, part: &mut FigurePart, depth: usize) -> Result<()> {
        let saved = std::mem::replace(&mut self.tokens, super::reverse_tokens(tokens));
        let result = self.scan_figure_body(None, part, depth);
        self.tokens = saved;
        result
    }

    /// Number the figure(s), register labels, and build the output blocks.
    fn assemble_figure(&mut self, mut part: FigurePart) -> Option<Block> {
        let mut blocks = Vec::new();

        // Separately captioned minipages come first, each a figure of its own
        for separate in std::mem::take(&mut part.separate) {
            if let Some(block) = self.assemble_figure(separate) {
                blocks.push(block);
            }
        }

        let has_content = !part.images.is_empty() || !part.subs.is_empty() || !part.blocks.is_empty();
        if !has_content && part.caption.is_none() {
            return finish(blocks);
        }

        // \caption steps the figure counter; sub-figures are numbered within
        let number = if part.caption.is_some() || !part.subs.is_empty() {
            Some(self.state.next_counter("figure"))
        } else {
            None
        };
        if let Some(label) = &part.label {
            let title = part
                .caption
                .as_ref()
                .map(|c| self.inlines_to_plain_text(c))
                .filter(|t| !t.is_empty());
            let number = number
                .clone()
                .unwrap_or_else(|| self.state.current_section_number.clone());
            self.state
                .add_label_with_number(label.clone(), LabelType::Figure, number, title);
        }

        for (i, mut sub) in std::mem::take(&mut part.subs).into_iter().enumerate() {
            let letter = alpha_number(i + 1, false);
            if let Some(label) = &sub.label {
                let n = format!("{}{}", number.clone().unwrap_or_default(), letter);
                let title = sub.caption.as_ref().map(|c| self.inlines_to_plain_text(c));
                self.state.add_label_with_number(label.clone(), LabelType::Figure, n, title);
            }
            let mut caption = vec![Inline::Text(format!("({})", letter))];
            if let Some(sub_caption) = sub.caption.take().filter(|c| !c.is_empty()) {
                caption.push(Inline::Space);
                caption.extend(sub_caption);
            }
            let label = sub.label.take();
            blocks.append(&mut sub.blocks);
            let mut images = sub.images.into_iter();
            match images.next() {
                Some(first) => {
                    blocks.push(Block::Figure {
                        caption: Some(caption),
                        path: first,
                        label,
                    });
                    for path in images {
                        blocks.push(Block::Figure {
                            caption: None,
                            path,
                            label: None,
                        });
                    }
                }
                None => blocks.push(Block::Paragraph(caption)),
            }
        }

        blocks.append(&mut part.blocks);
        let had_subs = blocks.iter().any(|b| matches!(b, Block::Figure { .. }));
        let mut caption = part.caption.take();
        let mut label = part.label.take();
        if !had_subs {
            let mut images = part.images.into_iter();
            if let Some(first) = images.next() {
                blocks.push(Block::Figure {
                    caption: caption.take(),
                    path: first,
                    label: label.take(),
                });
                for path in images {
                    blocks.push(Block::Figure {
                        caption: None,
                        path,
                        label: None,
                    });
                }
            }
        } else {
            for path in part.images {
                blocks.push(Block::Figure {
                    caption: None,
                    path,
                    label: None,
                });
            }
        }
        // A caption not attached to an image (sub-figure groups, TikZ-only
        // figures) is shown as its own line
        if let Some(caption) = caption.filter(|c| !c.is_empty()) {
            let mut line = Vec::new();
            if let Some(n) = &number {
                line.push(Inline::Strong(vec![Inline::Text(format!("Figure {}:", n))]));
                line.push(Inline::Space);
            }
            line.extend(caption);
            blocks.push(Block::Paragraph(line));
        }
        let _ = label;
        finish(blocks)
    }
}

fn merge_into(part: &mut FigurePart, sub: FigurePart) {
    part.images.extend(sub.images);
    part.blocks.extend(sub.blocks);
    part.subs.extend(sub.subs);
    if part.label.is_none() {
        part.label = sub.label;
    }
}

fn finish(mut blocks: Vec<Block>) -> Option<Block> {
    match blocks.len() {
        0 => None,
        1 => blocks.pop(),
        _ => Some(Block::Composite(blocks)),
    }
}
