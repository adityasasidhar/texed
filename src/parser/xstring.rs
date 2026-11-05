//! TeX boxes (`lrbox`, `\sbox`, `\savebox`, `\setbox` ... `\usebox`,
//! `\unhbox`) and the xstring package's string operations.
//!
//! Boxes are modelled as stored token lists: saving typesets nothing, using
//! the box re-inserts its content. xstring functions operate on the
//! expanded argument text, optionally storing the result in a macro
//! (`\StrSubstitute{abc}{b}{x}[\result]`).

use super::Parser;
use crate::error::Result;
use crate::lexer::Token;
use crate::state::MacroDef;

impl Parser {
    /// Box and xstring commands. `Ok(Some(()))` when handled.
    pub(super) fn parse_box_or_xstring(&mut self, cmd: &str) -> Result<Option<()>> {
        match cmd {
            // \sbox{\name}{content}, \savebox{\name}[w][pos]{content}
            "sbox" | "savebox" | "setbox@braced" => {
                self.skip_whitespace_and_comments();
                let name = self.box_name()?;
                while matches!(self.current_token(), Token::LeftBracket) {
                    let _ = self.collect_bracketed_tokens();
                }
                self.skip_whitespace_and_comments();
                let content = self.collect_braced_tokens();
                let content = self.resolve_box_uses(content);
                self.boxes.insert(name, content);
                Ok(Some(()))
            }
            // \usebox{\name}, \unhbox\name, \box\name, \copy\name
            "usebox" | "unhbox" | "unvbox" | "box" | "copy" | "unhcopy" | "unvcopy" => {
                self.skip_whitespace_and_comments();
                let name = self.box_name()?;
                let consume = matches!(cmd, "unhbox" | "unvbox" | "box");
                let content = if consume {
                    self.boxes.remove(&name)
                } else {
                    self.boxes.get(&name).cloned()
                };
                if let Some(content) = content {
                    if self.consume_expansion_budget("usebox") {
                        self.push_front(content);
                    }
                }
                Ok(Some(()))
            }
            // xstring
            "StrSubstitute" | "StrDel" => {
                let (opt_count, args) = self.xstring_args(if cmd == "StrDel" { 2 } else { 3 })?;
                let _ = opt_count;
                let text = args[0].clone();
                let from = args[1].clone();
                let to = if cmd == "StrDel" { String::new() } else { args[2].clone() };
                let result = if from.is_empty() { text } else { text.replace(&from, &to) };
                self.xstring_result(result)?;
                Ok(Some(()))
            }
            "StrLeft" | "StrRight" | "StrGobbleLeft" | "StrGobbleRight" | "StrChar" => {
                let (_, args) = self.xstring_args(2)?;
                let chars: Vec<char> = args[0].chars().collect();
                let n = args[1].trim().parse::<usize>().unwrap_or(0).min(chars.len());
                let result: String = match cmd {
                    "StrLeft" => chars[..n].iter().collect(),
                    "StrRight" => chars[chars.len() - n..].iter().collect(),
                    "StrGobbleLeft" => chars[n..].iter().collect(),
                    "StrGobbleRight" => chars[..chars.len() - n].iter().collect(),
                    _ => chars.get(n.saturating_sub(1)).map(|c| c.to_string()).unwrap_or_default(),
                };
                self.xstring_result(result)?;
                Ok(Some(()))
            }
            "StrMid" => {
                let (_, args) = self.xstring_args(3)?;
                let chars: Vec<char> = args[0].chars().collect();
                let a = args[1].trim().parse::<usize>().unwrap_or(1).max(1);
                let b = args[2].trim().parse::<usize>().unwrap_or(0).min(chars.len());
                let result: String = if a <= b { chars[a - 1..b].iter().collect() } else { String::new() };
                self.xstring_result(result)?;
                Ok(Some(()))
            }
            "StrBefore" | "StrBehind" => {
                let (_, args) = self.xstring_args(2)?;
                let result = match args[0].find(&args[1]) {
                    Some(pos) if !args[1].is_empty() => {
                        if cmd == "StrBefore" {
                            args[0][..pos].to_string()
                        } else {
                            args[0][pos + args[1].len()..].to_string()
                        }
                    }
                    _ => String::new(),
                };
                self.xstring_result(result)?;
                Ok(Some(()))
            }
            "StrLen" => {
                let (_, args) = self.xstring_args(1)?;
                let result = args[0].chars().count().to_string();
                self.xstring_result(result)?;
                Ok(Some(()))
            }
            "IfSubStr" | "IfStrEq" | "IfBeginWith" | "IfEndWith" | "IfEq" | "IfStrEqCase" => {
                if cmd == "IfStrEqCase" {
                    return Ok(None);
                }
                let (_, args) = self.xstring_args(2)?;
                let (a, b) = (args[0].trim(), args[1].trim());
                let value = match cmd {
                    "IfSubStr" => a.contains(b),
                    "IfStrEq" => a == b,
                    "IfBeginWith" => a.starts_with(b),
                    "IfEndWith" => a.ends_with(b),
                    _ => match (a.parse::<f64>(), b.parse::<f64>()) {
                        (Ok(x), Ok(y)) => (x - y).abs() < f64::EPSILON,
                        _ => a == b,
                    },
                };
                let yes = self.take_branch_tokens();
                let no = self.take_branch_tokens();
                self.push_front(if value { yes } else { no });
                Ok(Some(()))
            }
            // \tokenize\name{text}: \name holds the re-tokenized text
            "tokenize" => {
                self.skip_whitespace_and_comments();
                let name = match self.current_token().clone() {
                    Token::Command(n) => {
                        self.advance();
                        n
                    }
                    _ => return Ok(Some(())),
                };
                self.skip_whitespace_and_comments();
                let raw = self.parse_braced_text()?;
                let body = self.expand_parameterless_macros(&raw);
                self.state.macros.insert(
                    name,
                    MacroDef {
                        body,
                        ..Default::default()
                    },
                );
                Ok(Some(()))
            }
            _ => Ok(None),
        }
    }

    /// Box register name: `\name`, `{\name}` or a number.
    fn box_name(&mut self) -> Result<String> {
        Ok(match self.current_token().clone() {
            Token::Command(name) => {
                self.advance();
                name
            }
            Token::LeftBrace => {
                let text = self.parse_braced_text()?;
                text.trim().trim_start_matches('\\').to_string()
            }
            Token::Text(t) => {
                self.advance();
                let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
                let rest = t[digits.len()..].to_string();
                if !rest.is_empty() {
                    self.push_front(vec![Token::Text(rest)]);
                }
                digits
            }
            _ => String::new(),
        })
    }

    /// Replace `\usebox`/`\unhbox` references to stored boxes inside content
    /// being saved, so the saved box holds what was typeset at that moment.
    pub(super) fn resolve_box_uses(&self, tokens: Vec<Token>) -> Vec<Token> {
        let mut out = Vec::with_capacity(tokens.len());
        let mut iter = tokens.into_iter().peekable();
        while let Some(token) = iter.next() {
            if let Token::Command(c) = &token {
                if matches!(c.as_str(), "usebox" | "unhbox" | "unvbox" | "box" | "copy" | "unhcopy") {
                    // the register: \name or {\name}
                    let name = match iter.peek() {
                        Some(Token::Command(n)) => Some(n.clone()),
                        _ => None,
                    };
                    if let Some(name) = name {
                        if let Some(content) = self.boxes.get(&name) {
                            iter.next();
                            out.extend(content.iter().cloned());
                            continue;
                        }
                    }
                }
            }
            out.push(token);
        }
        out
    }

    /// Read `n` mandatory xstring arguments (expanded to text, as xstring
    /// does by default) and an optional trailing `[\name]`.
    fn xstring_args(&mut self, n: usize) -> Result<(usize, Vec<String>)> {
        self.consume_text_star();
        let mut args = Vec::new();
        for _ in 0..n {
            self.skip_whitespace_and_comments();
            let raw = self.parse_braced_text()?;
            args.push(self.expand_parameterless_macros(&raw));
        }
        // xstring's [n] occurrence argument
        let mut optional = 0;
        while matches!(self.current_token(), Token::LeftBracket) {
            let peek_is_name = matches!(self.peek_at(1), Some(Token::Command(_)));
            if peek_is_name {
                break;
            }
            let _ = self.collect_bracketed_tokens();
            optional += 1;
        }
        Ok((optional, args))
    }

    /// Deliver an xstring result: into `[\name]` if given, else as text.
    fn xstring_result(&mut self, result: String) -> Result<()> {
        if matches!(self.current_token(), Token::LeftBracket)
            && matches!(self.peek_at(1), Some(Token::Command(_)))
        {
            self.advance();
            if let Token::Command(name) = self.current_token().clone() {
                self.advance();
                if matches!(self.current_token(), Token::RightBracket) {
                    self.advance();
                }
                self.state.macros.insert(
                    name,
                    MacroDef {
                        body: result,
                        ..Default::default()
                    },
                );
                return Ok(());
            }
        }
        if !result.is_empty() {
            self.push_front(vec![Token::Text(result)]);
        }
        Ok(())
    }

    /// `\begin{lrbox}{\name} ... \end{lrbox}`: save the content in a box.
    pub(super) fn parse_lrbox(&mut self) -> Result<()> {
        self.skip_whitespace_and_comments();
        let name = self.box_name()?;
        let mut content = Vec::new();
        let mut depth = 0usize;
        while !self.is_at_end() {
            match self.current_token() {
                Token::EndEnvironment(env) if env == "lrbox" && depth == 0 => {
                    self.advance();
                    break;
                }
                Token::BeginEnvironment(env) if env == "lrbox" => depth += 1,
                Token::EndEnvironment(env) if env == "lrbox" => depth -= 1,
                // the box ends with an enclosing environment (its \end{lrbox}
                // comes from a user environment's end code)
                Token::EndEnvironment(env) if self.state.environments.contains_key(env) => {
                    let env = env.clone();
                    let end_def = self.state.environments[&env].end_def.clone();
                    if end_def.contains("\\end{lrbox}") {
                        // expand it here: content up to \end{lrbox} is ours
                        self.advance();
                        let tokens = super::tex::lex_fragment(&end_def);
                        self.push_front(tokens);
                        continue;
                    }
                    break;
                }
                _ => {}
            }
            content.push(self.current_token().clone());
            self.advance();
        }
        let content = self.resolve_box_uses(content);
        self.boxes.insert(name, content);
        Ok(())
    }
}
