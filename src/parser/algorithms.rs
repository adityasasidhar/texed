//! Pseudocode: `algorithm` floats and the algorithmic/algorithmicx
//! (`\State`, `\If{}`...`\EndIf`), old algorithmic (`\STATE`, `\IF{}`) and
//! algorithm2e (`\If{cond}{body}`, `\;`) environments.
//!
//! Each pseudocode line becomes one Markdown line: bold keywords,
//! indentation with non-breaking spaces, inline math kept as math, and
//! optional line numbers, joined with hard line breaks.

use super::{Block, Inline, Parser};
use crate::error::Result;
use crate::lexer::Token;
use crate::state::LabelType;

/// A pseudocode line: indentation level and content.
type Line = (usize, Vec<Inline>);

struct Algo {
    lines: Vec<Line>,
    current: Vec<Inline>,
    indent: usize,
    caption: Option<Vec<Inline>>,
    label: Option<String>,
}

impl Algo {
    fn new() -> Self {
        Algo {
            lines: Vec::new(),
            current: Vec::new(),
            indent: 0,
            caption: None,
            label: None,
        }
    }

    fn flush(&mut self) {
        let content = super::trim_inline_whitespace(std::mem::take(&mut self.current));
        if !content.is_empty() {
            self.lines.push((self.indent, content));
        }
    }

    /// Emit a keyword line (`**if** cond **then**`) at the current indent.
    fn keyword_line(&mut self, parts: Vec<Inline>) {
        self.flush();
        self.lines.push((self.indent, parts));
    }

    fn open(&mut self, parts: Vec<Inline>) {
        self.keyword_line(parts);
        self.indent += 1;
    }

    fn close(&mut self, parts: Option<Vec<Inline>>) {
        self.flush();
        self.indent = self.indent.saturating_sub(1);
        if let Some(parts) = parts {
            self.lines.push((self.indent, parts));
        }
    }
}

fn kw(word: &str) -> Inline {
    Inline::Strong(vec![Inline::Text(word.to_string())])
}

fn line(parts: Vec<Inline>) -> Vec<Inline> {
    parts
}

impl Parser {
    /// `\begin{algorithm}[H] ... \end{algorithm}` (also algorithm2e's own
    /// `algorithm` environment, whose body is pseudocode directly).
    pub(super) fn parse_algorithm_float(&mut self, env_name: &str) -> Result<Option<Block>> {
        self.skip_whitespace_and_comments();
        if matches!(self.current_token(), Token::LeftBracket) {
            let _ = self.collect_bracketed_tokens();
        }
        let mut algo = Algo::new();
        let mut line_numbers = false;
        let mut blocks = Vec::new();
        while !self.is_at_end() {
            match self.current_token().clone() {
                Token::EndEnvironment(env) if env == env_name => {
                    self.advance();
                    break;
                }
                Token::BeginEnvironment(env)
                    if matches!(env.as_str(), "algorithmic" | "algorithmic*" | "algorithmicx") =>
                {
                    self.advance();
                    self.skip_whitespace_only();
                    if matches!(self.current_token(), Token::LeftBracket) {
                        let n = self.parse_bracketed_text()?;
                        line_numbers = n.trim().parse::<usize>().unwrap_or(0) > 0;
                    }
                    self.scan_pseudocode(Some(&env), &mut algo)?;
                }
                _ => {
                    // algorithm2e: the float body itself is pseudocode
                    self.scan_pseudocode_until(Some(env_name), &mut algo, true)?;
                    break;
                }
            }
        }
        algo.flush();
        let caption = algo.caption.take();
        let number = caption.as_ref().map(|_| {
            self.algorithm_counter += 1;
            self.algorithm_counter.to_string()
        });
        if let Some(label) = algo.label.take() {
            let title = caption.as_ref().map(|c| self.inlines_to_plain_text(c));
            self.state.add_label_with_number(
                label,
                LabelType::Other(String::from("Algorithm")),
                number.clone().unwrap_or_default(),
                title,
            );
        }
        if caption.is_some() || number.is_some() {
            let mut head = vec![kw(&format!(
                "Algorithm{}",
                number.as_ref().map(|n| format!(" {}", n)).unwrap_or_default()
            ))];
            if let Some(caption) = caption {
                head.push(Inline::Space);
                head.extend(caption);
            }
            blocks.push(Block::Paragraph(head));
        }
        if let Some(body) = render_lines(&algo.lines, line_numbers) {
            blocks.push(body);
        }
        Ok(match blocks.len() {
            0 => None,
            1 => blocks.pop(),
            _ => Some(Block::Composite(blocks)),
        })
    }

    /// A bare `algorithmic` environment (outside an algorithm float).
    pub(super) fn parse_algorithmic(&mut self, env_name: &str) -> Result<Option<Block>> {
        self.skip_whitespace_only();
        let mut line_numbers = false;
        if matches!(self.current_token(), Token::LeftBracket) {
            let n = self.parse_bracketed_text()?;
            line_numbers = n.trim().parse::<usize>().unwrap_or(0) > 0;
        }
        let mut algo = Algo::new();
        self.scan_pseudocode(Some(env_name), &mut algo)?;
        algo.flush();
        Ok(render_lines(&algo.lines, line_numbers))
    }

    fn scan_pseudocode(&mut self, end_env: Option<&str>, algo: &mut Algo) -> Result<()> {
        self.scan_pseudocode_until(end_env, algo, false)
    }

    /// Interpret pseudocode tokens up to `\end{end_env}` (or the end of the
    /// current token stream).
    fn scan_pseudocode_until(&mut self, end_env: Option<&str>, algo: &mut Algo, algorithm2e: bool) -> Result<()> {
        let mut guard = 0usize;
        while !self.is_at_end() && guard < 1_000_000 {
            guard += 1;
            let token = self.current_token().clone();
            match &token {
                Token::EndEnvironment(env) if Some(env.as_str()) == end_env => {
                    self.advance();
                    return Ok(());
                }
                Token::Label(label) => {
                    algo.label = Some(label.clone());
                    self.advance();
                }
                Token::ParBreak | Token::Newline => {
                    self.advance();
                    // algorithm2e lines end with \; but blank lines also separate
                    if algorithm2e && matches!(token, Token::ParBreak) {
                        algo.flush();
                    } else {
                        algo.current.push(Inline::Space);
                    }
                }
                // algorithm2e: `\\` (with an optional skip) also ends a line
                Token::Backslash if algorithm2e => {
                    self.advance();
                    if matches!(self.current_token(), Token::LeftBracket) {
                        let _ = self.collect_bracketed_tokens();
                    }
                    algo.flush();
                }
                Token::Command(cmd) => {
                    let cmd = cmd.clone();
                    if !self.pseudocode_command(&cmd, algo)? {
                        // Ordinary content inside a line
                        if let Some(inline) = self.parse_inline()? {
                            algo.current.push(inline);
                        }
                    }
                }
                Token::BeginEnvironment(_) | Token::MathEnv { .. } | Token::DisplayMath(_) => {
                    algo.flush();
                    if let Some(block) = self.parse_block()? {
                        // Nested blocks inside pseudocode: keep their text
                        if let Block::Paragraph(inlines) = block {
                            algo.current.extend(inlines);
                        } else if let Block::DisplayMath(math) = block {
                            algo.current.push(Inline::InlineMath(math));
                        }
                    }
                }
                Token::EndEnvironment(_) => self.advance(),
                _ => {
                    if let Some(inline) = self.parse_inline()? {
                        algo.current.push(inline);
                    }
                }
            }
        }
        Ok(())
    }

    /// Braced argument as inlines (for conditions, names, comments).
    fn algo_arg(&mut self) -> Result<Vec<Inline>> {
        self.skip_whitespace_only();
        if !matches!(self.current_token(), Token::LeftBrace) {
            return Ok(Vec::new());
        }
        let tokens = self.collect_braced_tokens();
        let (inlines, _) = self.parse_inline_fragment_tokens(tokens)?;
        Ok(super::trim_inline_whitespace(inlines))
    }

    /// Run an algorithm2e body argument `{...}` as nested pseudocode.
    fn algo_body(&mut self, algo: &mut Algo) -> Result<()> {
        self.skip_whitespace_only();
        if !matches!(self.current_token(), Token::LeftBrace) {
            return Ok(());
        }
        let tokens = self.collect_braced_tokens();
        let saved = std::mem::replace(&mut self.tokens, super::reverse_tokens(tokens));
        let saved_expansions = std::mem::take(&mut self.active_expansions);
        let result = self.scan_pseudocode_until(None, algo, true);
        self.tokens = saved;
        self.active_expansions = saved_expansions;
        algo.flush();
        result
    }

    /// Handle a pseudocode command; false if it is ordinary content.
    fn pseudocode_command(&mut self, cmd: &str, algo: &mut Algo) -> Result<bool> {
        let lower = cmd.to_lowercase();
        // Commands that exist under the same name in several packages
        // behave per package: algorithm2e uses brace bodies.
        let groups = following_group_count(self);

        match cmd {
            "caption" => {
                self.advance();
                self.consume_text_star();
                if matches!(self.current_token(), Token::LeftBracket) {
                    let _ = self.collect_bracketed_tokens();
                }
                let tokens = self.collect_braced_tokens();
                let (inlines, label) = self.parse_inline_fragment_tokens(tokens)?;
                algo.caption = Some(super::trim_inline_whitespace(inlines));
                if label.is_some() {
                    algo.label = label;
                }
                return Ok(true);
            }
            "State" | "STATE" | "Statex" | "STATEx" | "item" => {
                self.advance();
                algo.flush();
                return Ok(true);
            }
            ";" => {
                // algorithm2e line end
                self.advance();
                algo.flush();
                return Ok(true);
            }
            _ => {}
        }

        // algorithm2e brace forms: \If{c}{body}, \eIf{c}{then}{else}, ...
        let is_2e = matches!(
            cmd,
            "eIf" | "uIf" | "lIf" | "uElseIf" | "lElseIf" | "ElseIf" | "uElse" | "lElse" | "Else"
                | "ForEach" | "ForAll" | "lFor" | "lForEach" | "lWhile" | "lRepeat" | "Repeat"
                | "Switch" | "Case" | "uCase" | "lCase" | "Other" | "lOther" | "Begin" | "If"
                | "For" | "While" | "KwIn" | "KwOut" | "KwData" | "KwResult" | "KwRet" | "Return"
                | "tcp" | "tcc" | "tcp*" | "tcc*" | "SetKwFunction" | "SetKwProg" | "SetAlgoLined"
                | "SetAlgoNoLine" | "DontPrintSemicolon" | "LinesNumbered" | "SetKwInOut"
                | "SetKwData" | "SetKw" | "SetKwComment" | "SetAlgoVlined" | "SetAlgoNoEnd"
                | "LinesNotNumbered" | "SetKwRepeat" | "SetKwIF" | "SetKwFor" | "SetKwSwitch"
                | "SetKwBlock" | "SetKwHangingKw" | "SetArgSty" | "SetFuncSty" | "SetDataSty"
                | "SetCommentSty" | "SetKwSty" | "ResetInOut" | "SetNlSty" | "SetAlCapSty"
                | "Indp" | "Indm" | "BlankLine" | "nl" | "FuncSty" | "ArgSty" | "DataSty"
                | "KwSty" | "CommentSty" | "lnl" | "SetAlgorithmName" | "SetAlgoCaptionSeparator"
                | "SetInd" | "AlgoDisplayBlockMarkers" | "SetAlgoSkip" | "IncMargin" | "DecMargin"
        ) && match cmd {
            // Names shared with algpseudocode: algorithm2e passes the body
            // as a braced argument
            "If" | "For" | "While" | "ElseIf" | "ForAll" | "Repeat" => groups >= 2,
            "Else" => groups >= 1,
            "Return" => false,
            _ => true,
        };

        if is_2e {
            self.advance();
            match cmd {
                "eIf" | "uIf" | "If" | "lIf" => {
                    let cond = self.algo_arg()?;
                    let mut head = vec![kw("if"), Inline::Space];
                    head.extend(cond);
                    head.extend([Inline::Space, kw("then")]);
                    if cmd == "lIf" {
                        algo.flush();
                        algo.current.extend(head);
                        algo.current.push(Inline::Space);
                        self.algo_body_inline(algo)?;
                        algo.flush();
                        return Ok(true);
                    }
                    algo.open(head);
                    self.algo_body(algo)?;
                    if cmd == "eIf" {
                        algo.close(Some(line(vec![kw("else")])));
                        algo.indent += 1;
                        self.algo_body(algo)?;
                    }
                    algo.close(if cmd == "uIf" { None } else { Some(line(vec![kw("end")])) });
                }
                "uElseIf" | "ElseIf" | "lElseIf" => {
                    let cond = self.algo_arg()?;
                    let mut head = vec![kw("else if"), Inline::Space];
                    head.extend(cond);
                    head.extend([Inline::Space, kw("then")]);
                    algo.open(head);
                    self.algo_body(algo)?;
                    algo.close(if cmd == "ElseIf" { Some(line(vec![kw("end")])) } else { None });
                }
                "uElse" | "Else" | "lElse" => {
                    algo.open(vec![kw("else")]);
                    self.algo_body(algo)?;
                    algo.close(if cmd == "Else" { Some(line(vec![kw("end")])) } else { None });
                }
                "For" | "ForEach" | "ForAll" | "lFor" | "lForEach" | "While" | "lWhile" => {
                    let cond = self.algo_arg()?;
                    let word = match cmd {
                        "ForEach" | "lForEach" => "foreach",
                        "ForAll" => "forall",
                        "While" | "lWhile" => "while",
                        _ => "for",
                    };
                    let mut head = vec![kw(word), Inline::Space];
                    head.extend(cond);
                    head.extend([Inline::Space, kw("do")]);
                    if cmd.starts_with('l') {
                        algo.flush();
                        algo.current.extend(head);
                        algo.current.push(Inline::Space);
                        self.algo_body_inline(algo)?;
                        algo.flush();
                    } else {
                        algo.open(head);
                        self.algo_body(algo)?;
                        algo.close(Some(line(vec![kw("end")])));
                    }
                }
                "Repeat" | "lRepeat" => {
                    let cond = self.algo_arg()?;
                    algo.open(vec![kw("repeat")]);
                    self.algo_body(algo)?;
                    let mut tail = vec![kw("until"), Inline::Space];
                    tail.extend(cond);
                    algo.close(Some(tail));
                }
                "Switch" => {
                    let cond = self.algo_arg()?;
                    let mut head = vec![kw("switch"), Inline::Space];
                    head.extend(cond);
                    head.extend([Inline::Space, kw("do")]);
                    algo.open(head);
                    self.algo_body(algo)?;
                    algo.close(Some(line(vec![kw("end")])));
                }
                "Case" | "uCase" | "lCase" => {
                    let cond = self.algo_arg()?;
                    let mut head = vec![kw("case"), Inline::Space];
                    head.extend(cond);
                    algo.open(head);
                    self.algo_body(algo)?;
                    algo.close(None);
                }
                "Other" | "lOther" => {
                    algo.open(vec![kw("otherwise")]);
                    self.algo_body(algo)?;
                    algo.close(None);
                }
                "Begin" => {
                    algo.open(vec![kw("begin")]);
                    self.algo_body(algo)?;
                    algo.close(Some(line(vec![kw("end")])));
                }
                "KwIn" | "KwOut" | "KwData" | "KwResult" => {
                    let word = match cmd {
                        "KwIn" => "Input:",
                        "KwOut" => "Output:",
                        "KwData" => "Data:",
                        _ => "Result:",
                    };
                    let mut parts = vec![kw(word), Inline::Space];
                    parts.extend(self.algo_arg()?);
                    algo.keyword_line(parts);
                }
                "KwRet" | "Return" => {
                    algo.current.push(kw("return"));
                    algo.current.push(Inline::Space);
                    let value = self.algo_arg()?;
                    algo.current.extend(value);
                }
                "tcp" | "tcc" | "tcp*" | "tcc*" => {
                    let comment = self.algo_arg()?;
                    if !algo.current.is_empty() {
                        algo.current.push(Inline::Space);
                    }
                    algo.current.push(Inline::Text(String::from("// ")));
                    algo.current.push(Inline::Emph(comment));
                }
                "BlankLine" => {
                    algo.flush();
                }
                "Indp" => algo.indent += 1,
                "Indm" => algo.indent = algo.indent.saturating_sub(1),
                _ => {
                    // Configuration commands: consume their arguments
                    self.consume_trailing_groups(4)?;
                }
            }
            return Ok(true);
        }

        // algorithmicx / algorithmic keywords are capitalized (\If, \IF);
        // lowercase names are TeX/LaTeX commands (\if, \or, \and, \to)
        if !cmd.starts_with(|c: char| c.is_ascii_uppercase()) {
            return Ok(false);
        }
        let consumed = match lower.as_str() {
            "if" => {
                self.advance();
                let cond = self.algo_arg()?;
                let mut head = vec![kw("if"), Inline::Space];
                head.extend(cond);
                head.extend([Inline::Space, kw("then")]);
                algo.open(head);
                true
            }
            "elsif" | "elseif" => {
                self.advance();
                let cond = self.algo_arg()?;
                algo.close(None);
                let mut head = vec![kw("else if"), Inline::Space];
                head.extend(cond);
                head.extend([Inline::Space, kw("then")]);
                algo.open(head);
                true
            }
            "else" => {
                self.advance();
                algo.close(None);
                algo.open(vec![kw("else")]);
                true
            }
            "endif" => {
                self.advance();
                algo.close(Some(vec![kw("end if")]));
                true
            }
            "for" | "forall" | "foreach" | "while" => {
                self.advance();
                let cond = self.algo_arg()?;
                let word = match lower.as_str() {
                    "forall" => "for all",
                    "foreach" => "for each",
                    "while" => "while",
                    _ => "for",
                };
                let mut head = vec![kw(word), Inline::Space];
                head.extend(cond);
                head.extend([Inline::Space, kw("do")]);
                algo.open(head);
                true
            }
            "endfor" | "endwhile" | "endloop" | "endprocedure" | "endfunction" => {
                self.advance();
                let word = match lower.as_str() {
                    "endfor" => "end for",
                    "endwhile" => "end while",
                    "endloop" => "end loop",
                    "endprocedure" => "end procedure",
                    _ => "end function",
                };
                algo.close(Some(vec![kw(word)]));
                true
            }
            "repeat" => {
                self.advance();
                algo.open(vec![kw("repeat")]);
                true
            }
            "until" => {
                self.advance();
                let cond = self.algo_arg()?;
                let mut tail = vec![kw("until"), Inline::Space];
                tail.extend(cond);
                algo.close(Some(tail));
                true
            }
            "loop" => {
                self.advance();
                algo.open(vec![kw("loop")]);
                true
            }
            "procedure" | "function" => {
                self.advance();
                let name = self.algo_arg()?;
                let args = self.algo_arg()?;
                let mut head = vec![kw(&lower), Inline::Space, Inline::SmallCaps(name)];
                head.push(Inline::Text(String::from("(")));
                head.extend(args);
                head.push(Inline::Text(String::from(")")));
                algo.open(head);
                true
            }
            "require" | "ensure" | "input" | "output" => {
                self.advance();
                let word = match lower.as_str() {
                    "require" => "Require:",
                    "ensure" => "Ensure:",
                    "input" => "Input:",
                    _ => "Output:",
                };
                algo.flush();
                algo.current.push(kw(word));
                algo.current.push(Inline::Space);
                true
            }
            "return" | "print" => {
                self.advance();
                algo.current.push(kw(&lower));
                algo.current.push(Inline::Space);
                true
            }
            "comment" => {
                self.advance();
                let comment = self.algo_arg()?;
                let mut parts = vec![Inline::Text(String::from(" \u{25B7} "))];
                parts.extend(comment);
                // \While{c}\Comment{x}: the comment belongs to the line just opened
                match algo.lines.last_mut() {
                    Some((_, last)) if algo.current.is_empty() => last.extend(parts),
                    _ => algo.current.extend(parts),
                }
                true
            }
            "call" => {
                self.advance();
                let name = self.algo_arg()?;
                let args = self.algo_arg()?;
                algo.current.push(Inline::SmallCaps(name));
                algo.current.push(Inline::Text(String::from("(")));
                algo.current.extend(args);
                algo.current.push(Inline::Text(String::from(")")));
                true
            }
            "and" | "or" | "not" | "true" | "false" | "to" if cmd.chars().all(|c| c.is_uppercase()) => {
                self.advance();
                algo.current.push(kw(&lower));
                true
            }
            _ => false,
        };
        Ok(consumed)
    }

    /// algorithm2e one-line forms (`\lIf{c}{x}`): body inline on this line.
    fn algo_body_inline(&mut self, algo: &mut Algo) -> Result<()> {
        let mut nested = Algo::new();
        self.algo_body(&mut nested)?;
        for (i, (_, content)) in nested.lines.into_iter().enumerate() {
            if i > 0 {
                algo.current.push(Inline::Text(String::from("; ")));
            }
            algo.current.extend(content);
        }
        Ok(())
    }
}

/// Number of braced groups directly following the current command (the
/// current token, at offset 0).
fn following_group_count(p: &Parser) -> usize {
    let mut look = 1;
    let mut groups = 0;
    loop {
        while matches!(p.peek_at(look), Some(Token::Whitespace(_)) | Some(Token::Newline)) {
            look += 1;
        }
        if !matches!(p.peek_at(look), Some(Token::LeftBrace)) || groups >= 3 {
            return groups;
        }
        let mut depth = 0usize;
        loop {
            match p.peek_at(look) {
                Some(Token::LeftBrace) => depth += 1,
                Some(Token::RightBrace) => {
                    depth -= 1;
                    if depth == 0 {
                        look += 1;
                        break;
                    }
                }
                None => return groups,
                _ => {}
            }
            look += 1;
        }
        groups += 1;
    }
}

/// Render pseudocode lines as one paragraph with hard line breaks.
fn render_lines(lines: &[Line], numbers: bool) -> Option<Block> {
    if lines.is_empty() {
        return None;
    }
    let width = lines.len().to_string().len();
    let mut inlines = Vec::new();
    for (i, (indent, content)) in lines.iter().enumerate() {
        if i > 0 {
            inlines.push(Inline::LineBreak);
        }
        if numbers {
            inlines.push(Inline::Text(format!("{:>width$}:\u{00A0}", i + 1, width = width)));
        }
        if *indent > 0 {
            inlines.push(Inline::Text("\u{00A0}".repeat(4 * indent)));
        }
        inlines.extend(content.iter().cloned());
    }
    Some(Block::Paragraph(inlines))
}
