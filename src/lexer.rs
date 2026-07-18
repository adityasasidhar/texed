use crate::error::{Result, TexedError};
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Commands
    Command(String),              // \command
    BeginEnvironment(String),     // \begin{env}
    EndEnvironment(String),       // \end{env}
    
    // Text and content
    Text(String),
    Whitespace(String),
    Newline,
    ParBreak,                     // Double newline
    
    // Delimiters
    LeftBrace,                    // {
    RightBrace,                   // }
    LeftBracket,                  // [
    RightBracket,                 // ]
    
    // Math mode
    InlineMath(String),           // $...$ or \(...\)
    DisplayMath(String),          // $$...$$ or \[...\]

    // Verbatim
    Verb(String),                 // \verb|...| / \lstinline|...|
    VerbatimEnv { name: String, content: String }, // verbatim/lstlisting/minted captured raw
    
    // Special characters
    Ampersand,                    // & (table separator)
    Backslash,                    // \\
    Tilde,                        // ~ (non-breaking space)
    Percent,                      // % (comment start)
    Comment(String),
    
    // Sectioning
    Section(u8, String),          // Level and title
    
    // Lists
    Item,                         // \item
    
    // Special commands
    Label(String),
    Ref { kind: String, label: String },
    Cite { kind: String, keys: String },
    
    // End of input
    Eof,
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Command(cmd) => write!(f, "\\{}", cmd),
            Token::Text(text) => write!(f, "{}", text),
            Token::BeginEnvironment(env) => write!(f, "\\begin{{{}}}", env),
            Token::EndEnvironment(env) => write!(f, "\\end{{{}}}", env),
            _ => write!(f, "{:?}", self),
        }
    }
}

pub struct Lexer {
    input: Vec<char>,
    position: usize,
    line: usize,
    column: usize,
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        Self {
            input: input.chars().collect(),
            position: 0,
            line: 1,
            column: 1,
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>> {
        let mut tokens = Vec::new();

        while !self.is_at_end() {
            match self.next_token()? {
                Some(token) => tokens.push(token),
                None => continue,
            }
        }

        tokens.push(Token::Eof);
        Ok(tokens)
    }

    fn next_token(&mut self) -> Result<Option<Token>> {
        // Don't skip whitespace here - let it be tokenized as Whitespace tokens
        
        if self.is_at_end() {
            return Ok(None);
        }

        let ch = self.current_char();
        
        // Handle smart quotes
        if ch == '`' {
            if self.peek_char() == Some('`') {
                self.advance();
                self.advance();
                return Ok(Some(Token::Text("\u{201C}".to_string()))); // Left double quote
            } else {
                self.advance();
                return Ok(Some(Token::Text("\u{2018}".to_string()))); // Left single quote
            }
        }
        
        if ch == '\'' {
            if self.peek_char() == Some('\'') {
                self.advance();
                self.advance();
                return Ok(Some(Token::Text("\u{201D}".to_string()))); // Right double quote
            } else {
                self.advance();
                return Ok(Some(Token::Text("\u{2019}".to_string()))); // Right single quote
            }
        }

        match ch {
            '\\' => self.lex_command(),
            '{' => {
                self.advance();
                Ok(Some(Token::LeftBrace))
            }
            '}' => {
                self.advance();
                Ok(Some(Token::RightBrace))
            }
            '[' => {
                self.advance();
                Ok(Some(Token::LeftBracket))
            }
            ']' => {
                self.advance();
                Ok(Some(Token::RightBracket))
            }
            '$' => self.lex_math(),
            '%' => self.lex_comment(),
            '&' => {
                self.advance();
                Ok(Some(Token::Ampersand))
            }
            '~' => {
                self.advance();
                Ok(Some(Token::Tilde))
            }
            '\n' => self.lex_newline(),
            ' ' | '\t' | '\r' => self.lex_whitespace(),
            _ => self.lex_text(),
        }
    }

    fn lex_command(&mut self) -> Result<Option<Token>> {
        self.advance(); // consume '\'

        if self.is_at_end() {
            return Ok(Some(Token::Backslash));
        }

        let ch = self.current_char();

        // Handle special backslash sequences
        if ch == '\\' {
            self.advance();
            return Ok(Some(Token::Backslash));
        }

        if ch == '[' {
            self.advance();
            return self.lex_display_math_bracket();
        }

        if ch == ']' {
            self.advance();
            return Ok(Some(Token::Text(String::from("\\]"))));
        }

        // \( ... \) inline math
        if ch == '(' {
            self.advance();
            let content = self.read_until_sequence("\\)");
            return Ok(Some(Token::InlineMath(content)));
        }
        if ch == ')' {
            self.advance();
            return Ok(None);
        }

        // Escaped literal characters: \% \$ \& \# \_ \{ \}
        if matches!(ch, '%' | '$' | '&' | '#' | '_' | '{' | '}') {
            self.advance();
            return Ok(Some(Token::Text(ch.to_string())));
        }

        // Accent and spacing control symbols: emitted as single-char commands,
        // resolved by the parser (\' \` \^ \" \~ \= \. accents; \, \; \: \! spacing)
        if matches!(ch, '\'' | '`' | '^' | '"' | '~' | '=' | '.' | ',' | ';' | ':' | '!') {
            self.advance();
            return Ok(Some(Token::Command(ch.to_string())));
        }

        // Explicit interword space: "\ " or backslash before line end
        if ch == ' ' || ch == '\t' || ch == '\n' {
            self.advance();
            return Ok(Some(Token::Whitespace(String::from(" "))));
        }

        // Discretionary hyphen \-, italic correction \/, spacing factor \@: no output
        if matches!(ch, '-' | '/' | '@' | '*') {
            self.advance();
            return Ok(None);
        }

        // Lex command name
        let cmd_name = self.lex_command_name();

        if cmd_name.is_empty() {
            // Unknown control symbol: keep the symbol itself as text
            self.advance();
            return Ok(Some(Token::Text(ch.to_string())));
        }

        // \verb|...| and friends capture raw content with arbitrary delimiters
        if cmd_name == "verb" || cmd_name == "lstinline" || cmd_name == "Verb" {
            return self.lex_verb();
        }

        // Handle specific commands
        match cmd_name.as_str() {
            "begin" => self.lex_begin_environment(),
            "end" => self.lex_end_environment(),
            "item" => Ok(Some(Token::Item)),
            "label" => self.lex_label(),
            "ref" | "eqref" | "autoref" | "nameref" | "pageref" | "cref" | "Cref" | "vref" => {
                self.lex_ref(&cmd_name)
            }
            "cite" | "citep" | "citet" | "citealt" | "citealp" | "citeauthor" | "citeyear"
            | "citeyearpar" | "autocite" | "textcite" | "parencite" | "footcite" => {
                self.lex_cite(&cmd_name)
            }
            "section" => self.lex_section(1),
            "subsection" => self.lex_section(2),
            "subsubsection" => self.lex_section(3),
            "chapter" => self.lex_section(0),
            "part" => self.lex_section(0),
            "paragraph" => self.lex_section(4),
            "subparagraph" => self.lex_section(5),
            _ => Ok(Some(Token::Command(cmd_name))),
        }
    }

    fn lex_verb(&mut self) -> Result<Option<Token>> {
        // Optional star variant (\verb*)
        if !self.is_at_end() && self.current_char() == '*' {
            self.advance();
        }

        if self.is_at_end() {
            return Ok(Some(Token::Command(String::from("verb"))));
        }

        let delim = self.current_char();
        self.advance();
        let close = if delim == '{' { '}' } else { delim };

        let mut content = String::new();
        while !self.is_at_end() && self.current_char() != close {
            content.push(self.current_char());
            self.advance();
        }
        if !self.is_at_end() {
            self.advance(); // consume closing delimiter
        }

        Ok(Some(Token::Verb(content)))
    }

    fn lex_command_name(&mut self) -> String {
        let mut name = String::new();

        while !self.is_at_end() {
            let ch = self.current_char();
            if ch.is_alphabetic() {
                name.push(ch);
                self.advance();
            } else {
                break;
            }
        }

        name
    }

    fn lex_begin_environment(&mut self) -> Result<Option<Token>> {
        // Don't skip whitespace - let it be tokenized separately

        if self.is_at_end() || self.current_char() != '{' {
            return Ok(Some(Token::Command(String::from("begin"))));
        }

        self.advance(); // consume '{'
        let env_name = self.read_until('}');

        if self.is_at_end() || self.current_char() != '}' {
            return Err(TexedError::UnbalancedBraces);
        }

        self.advance(); // consume '}'

        // Verbatim-like environments capture their body raw so that %, $, \ etc.
        // inside code are not interpreted as LaTeX.
        if is_verbatim_environment(&env_name) {
            let content = self.read_until_sequence(&format!("\\end{{{}}}", env_name));
            return Ok(Some(Token::VerbatimEnv {
                name: env_name,
                content,
            }));
        }

        Ok(Some(Token::BeginEnvironment(env_name)))
    }

    fn lex_end_environment(&mut self) -> Result<Option<Token>> {
        // Don't skip whitespace - let it be tokenized separately
        
        if self.current_char() != '{' {
            return Ok(Some(Token::Command(String::from("end"))));
        }

        self.advance(); // consume '{'
        let env_name = self.read_until('}');
        
        if self.is_at_end() || self.current_char() != '}' {
            return Err(TexedError::UnbalancedBraces);
        }
        
        self.advance(); // consume '}'
        Ok(Some(Token::EndEnvironment(env_name)))
    }

    fn lex_section(&mut self, level: u8) -> Result<Option<Token>> {
        // Starred variant (\section*) — unnumbered, same heading
        if !self.is_at_end() && self.current_char() == '*' {
            self.advance();
        }

        if self.is_at_end() {
            return Ok(Some(Token::Command(String::from("section"))));
        }

        // Handle optional argument [short title]
        if self.current_char() == '[' {
            self.advance();
            let _ = self.read_until(']');
            if !self.is_at_end() && self.current_char() == ']' {
                self.advance();
            }
            self.skip_whitespace();
        }

        // Read section title (brace-aware: titles may contain nested groups)
        if self.is_at_end() || self.current_char() != '{' {
            return Ok(Some(Token::Command(String::from("section"))));
        }

        self.advance(); // consume '{'
        let title = self.read_balanced_braces_content();

        Ok(Some(Token::Section(level, title)))
    }

    /// Read content up to the matching closing brace, assuming the opening
    /// brace has already been consumed. Handles nesting and escaped braces.
    fn read_balanced_braces_content(&mut self) -> String {
        let mut content = String::new();
        let mut depth: usize = 1;

        while !self.is_at_end() {
            let ch = self.current_char();

            if ch == '\\' {
                content.push(ch);
                self.advance();
                if !self.is_at_end() {
                    content.push(self.current_char());
                    self.advance();
                }
                continue;
            }

            if ch == '{' {
                depth += 1;
            } else if ch == '}' {
                depth -= 1;
                if depth == 0 {
                    self.advance();
                    break;
                }
            }

            content.push(ch);
            self.advance();
        }

        content
    }

    fn lex_label(&mut self) -> Result<Option<Token>> {
        // Don't skip whitespace - let it be tokenized separately
        
        if self.current_char() != '{' {
            return Ok(Some(Token::Command(String::from("label"))));
        }

        self.advance();
        let label = self.read_until('}');
        
        if !self.is_at_end() && self.current_char() == '}' {
            self.advance();
        }

        Ok(Some(Token::Label(label)))
    }

    fn lex_ref(&mut self, kind: &str) -> Result<Option<Token>> {
        // Don't skip whitespace - let it be tokenized separately
        
        if self.current_char() != '{' {
            return Ok(Some(Token::Command(kind.to_string())));
        }

        self.advance();
        let reference = self.read_until('}');
        
        if !self.is_at_end() && self.current_char() == '}' {
            self.advance();
        }

        Ok(Some(Token::Ref {
            kind: kind.to_string(),
            label: reference,
        }))
    }

    fn lex_cite(&mut self, kind: &str) -> Result<Option<Token>> {
        // Don't skip whitespace - let it be tokenized separately
        
        if self.current_char() != '{' {
            return Ok(Some(Token::Command(kind.to_string())));
        }

        self.advance();
        let citation = self.read_until('}');
        
        if !self.is_at_end() && self.current_char() == '}' {
            self.advance();
        }

        Ok(Some(Token::Cite {
            kind: kind.to_string(),
            keys: citation,
        }))
    }

    fn lex_math(&mut self) -> Result<Option<Token>> {
        self.advance(); // consume first '$'

        if !self.is_at_end() && self.current_char() == '$' {
            // Display math $$...$$
            self.advance(); // consume second '$'
            let content = self.read_until_double_dollar();
            return Ok(Some(Token::DisplayMath(content)));
        }

        // Inline math $...$
        let content = self.read_until('$');
        
        if !self.is_at_end() && self.current_char() == '$' {
            self.advance();
        }

        Ok(Some(Token::InlineMath(content)))
    }

    fn lex_display_math_bracket(&mut self) -> Result<Option<Token>> {
        let content = self.read_until_sequence("\\]");
        Ok(Some(Token::DisplayMath(content)))
    }

    fn read_until_double_dollar(&mut self) -> String {
        let mut content = String::new();
        let mut prev_was_dollar = false;

        while !self.is_at_end() {
            let ch = self.current_char();
            
            if ch == '$' && prev_was_dollar {
                self.advance();
                break;
            }

            if ch == '$' {
                prev_was_dollar = true;
            } else {
                if prev_was_dollar {
                    content.push('$');
                }
                prev_was_dollar = false;
                content.push(ch);
            }

            self.advance();
        }

        content
    }

    fn read_until_sequence(&mut self, sequence: &str) -> String {
        let mut content = String::new();
        let seq_chars: Vec<char> = sequence.chars().collect();

        while !self.is_at_end() {
            if self.matches_sequence(&seq_chars) {
                for _ in 0..seq_chars.len() {
                    self.advance();
                }
                break;
            }

            content.push(self.current_char());
            self.advance();
        }

        content
    }

    fn matches_sequence(&self, sequence: &[char]) -> bool {
        for (i, &ch) in sequence.iter().enumerate() {
            if self.position + i >= self.input.len() {
                return false;
            }
            if self.input[self.position + i] != ch {
                return false;
            }
        }
        true
    }

    fn lex_comment(&mut self) -> Result<Option<Token>> {
        self.advance(); // consume '%'
        let comment = self.read_until('\n');

        // In TeX, % consumes the line terminator too, joining the lines.
        // But keep the newline when a blank line follows, so paragraph
        // breaks around full-line comments are preserved.
        if !self.is_at_end() && self.current_char() == '\n' {
            let mut lookahead = self.position + 1;
            while lookahead < self.input.len()
                && matches!(self.input[lookahead], ' ' | '\t' | '\r')
            {
                lookahead += 1;
            }
            let blank_line_follows =
                lookahead >= self.input.len() || self.input[lookahead] == '\n';
            if !blank_line_follows {
                self.advance();
            }
        }

        Ok(Some(Token::Comment(comment)))
    }

    fn lex_whitespace(&mut self) -> Result<Option<Token>> {
        let mut whitespace = String::new();
        
        while !self.is_at_end() {
            let ch = self.current_char();
            if ch == ' ' || ch == '\t' || ch == '\r' {
                whitespace.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        
        if whitespace.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Token::Whitespace(whitespace)))
        }
    }

    fn lex_newline(&mut self) -> Result<Option<Token>> {
        self.advance();
        
        // Check for paragraph break (double newline)
        self.skip_whitespace_except_newlines();
        
        if !self.is_at_end() && self.current_char() == '\n' {
            self.advance();
            return Ok(Some(Token::ParBreak));
        }

        Ok(Some(Token::Newline))
    }

    fn lex_text(&mut self) -> Result<Option<Token>> {
        let mut text = String::new();

        while !self.is_at_end() {
            let ch = self.current_char();

            if ch == '\\' || ch == '{' || ch == '}' || ch == '[' || ch == ']'
                || ch == '$' || ch == '%' || ch == '&' || ch == '~' || ch == '\n'
                || ch == ' ' || ch == '\t' || ch == '\r' || ch == '\'' || ch == '`' {
                break;
            }

            text.push(ch);
            self.advance();
        }

        if text.is_empty() {
            Ok(None)
        } else {
            // TeX ligatures: --- em dash, -- en dash
            if text.contains("--") {
                text = text.replace("---", "\u{2014}").replace("--", "\u{2013}");
            }
            Ok(Some(Token::Text(text)))
        }
    }

    fn read_until(&mut self, delimiter: char) -> String {
        let mut content = String::new();

        while !self.is_at_end() {
            let ch = self.current_char();
            
            if ch == delimiter {
                break;
            }

            if ch == '\\' {
                content.push(ch);
                self.advance();
                if !self.is_at_end() {
                    content.push(self.current_char());
                    self.advance();
                }
            } else {
                content.push(ch);
                self.advance();
            }
        }

        content
    }

    fn current_char(&self) -> char {
        self.input[self.position]
    }
    
    fn peek_char(&self) -> Option<char> {
        if self.position + 1 < self.input.len() {
            Some(self.input[self.position + 1])
        } else {
            None
        }
    }

    fn advance(&mut self) {
        if !self.is_at_end() {
            if self.input[self.position] == '\n' {
                self.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
            self.position += 1;
        }
    }

    fn skip_whitespace(&mut self) {
        while !self.is_at_end() {
            let ch = self.current_char();
            if ch == ' ' || ch == '\t' || ch == '\r' || ch == '\n' {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn skip_whitespace_except_newlines(&mut self) {
        while !self.is_at_end() {
            let ch = self.current_char();
            if ch == ' ' || ch == '\t' || ch == '\r' {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn is_at_end(&self) -> bool {
        self.position >= self.input.len()
    }
}

/// Environments whose body must be captured raw at lex time so LaTeX special
/// characters inside them (%, $, &, \, ...) are not interpreted.
pub fn is_verbatim_environment(name: &str) -> bool {
    matches!(
        name,
        "verbatim"
            | "verbatim*"
            | "Verbatim"
            | "BVerbatim"
            | "lstlisting"
            | "minted"
            | "alltt"
            | "comment"
            | "filecontents"
            | "filecontents*"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_text() {
        let mut lexer = Lexer::new("Hello World");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens.len(), 4); // Text + Whitespace + Text + Eof
    }

    #[test]
    fn test_command() {
        let mut lexer = Lexer::new("\\textbf{bold}");
        let tokens = lexer.tokenize().unwrap();
        assert!(matches!(tokens[0], Token::Command(_)));
    }

    #[test]
    fn test_environment() {
        let mut lexer = Lexer::new("\\begin{document}\\end{document}");
        let tokens = lexer.tokenize().unwrap();
        assert!(matches!(tokens[0], Token::BeginEnvironment(_)));
        assert!(matches!(tokens[1], Token::EndEnvironment(_)));
    }

    #[test]
    fn test_inline_math() {
        let mut lexer = Lexer::new("$x + y$");
        let tokens = lexer.tokenize().unwrap();
        assert!(matches!(tokens[0], Token::InlineMath(_)));
    }

    #[test]
    fn test_display_math() {
        let mut lexer = Lexer::new("$$E = mc^2$$");
        let tokens = lexer.tokenize().unwrap();
        assert!(matches!(tokens[0], Token::DisplayMath(_)));
    }
}
