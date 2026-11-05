use crate::error::Result;
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
    /// A display-math environment (equation, align, ...) with its body
    /// captured raw so text-mode ligatures/quotes never touch the math.
    MathEnv { name: String, content: String },
    
    // Special characters
    Ampersand,                    // & (table separator)
    Backslash,                    // \\
    Tilde,                        // ~ (non-breaking space)
    Percent,                      // % (comment start)
    Comment(String),
    
    // Sectioning
    /// Sectioning command: level (0 = part/chapter ... 5 = subparagraph),
    /// title, and flags.
    Section(u8, String, SectionFlags),
    
    // Lists
    Item,                         // \item
    
    // Special commands
    Label(String),
    Ref { kind: String, label: String },
    Cite { kind: String, keys: String },
    
    // End of input
    Eof,
}

/// Variant flags of a sectioning command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SectionFlags {
    /// `\section*`: unnumbered, does not step counters
    pub starred: bool,
    /// `\part`: has its own counter, does not step chapters
    pub part: bool,
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
    /// `@` is a letter in command names (after `\makeatletter`, and in
    /// package/class files)
    at_letter: bool,
    /// expl3 syntax (`\ExplSyntaxOn`): `_` and `:` are letters
    expl_syntax: bool,
    /// Lexing math content: no text ligatures or smart quotes, escaped
    /// characters keep their backslash, no raw environment capture.
    math_mode: bool,
    /// Tokens produced by a multi-token construct, emitted before lexing
    /// continues (stored in emission order).
    pending: Vec<Token>,
    /// Environments defined as verbatim by the document (\lstnewenvironment,
    /// \DefineVerbatimEnvironment, \newminted, ...).
    verbatim_envs: std::collections::HashSet<String>,
    /// Commands defined as inline verbatim (\newmintinline[ltx]{latex}{}).
    verb_commands: std::collections::HashSet<String>,
    /// Characters made inline-verbatim delimiters (\MakeShortVerb{\|}),
    /// active from their definition on.
    short_verb_chars: std::collections::HashSet<char>,
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        let verbatim_envs = scan_verbatim_env_definitions(input)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        let verb_commands = scan_verb_command_definitions(input).into_iter().collect();
        Self {
            verbatim_envs,
            verb_commands,
            short_verb_chars: std::collections::HashSet::new(),
            input: input.chars().collect(),
            position: 0,
            line: 1,
            column: 1,
            at_letter: false,
            expl_syntax: false,
            math_mode: false,
            pending: Vec::new(),
        }
    }

    /// Lex math content (for macro expansion inside formulas): the
    /// resulting tokens serialize back to exactly the original math.
    pub fn with_math_mode(mut self, math_mode: bool) -> Self {
        self.math_mode = math_mode;
        self
    }

    /// Additional environments whose bodies are verbatim (defined elsewhere
    /// in the document, e.g. in a previously read file).
    pub fn with_verbatim_envs<'a>(mut self, names: impl IntoIterator<Item = &'a String>) -> Self {
        self.verbatim_envs.extend(names.into_iter().cloned());
        self
    }

    /// Additional inline-verbatim commands defined elsewhere in the document.
    pub fn with_verb_commands<'a>(mut self, names: impl IntoIterator<Item = &'a String>) -> Self {
        for name in names {
            // short-verb characters from the document's packages
            match name.strip_prefix("texed@shortverb:").and_then(|c| c.chars().next()) {
                Some(c) => {
                    self.short_verb_chars.insert(c);
                }
                None => {
                    self.verb_commands.insert(name.clone());
                }
            }
        }
        self
    }

    /// Start in expl3 syntax (`_` and `:` are letters).
    pub fn with_expl_syntax(mut self, expl: bool) -> Self {
        self.expl_syntax = expl;
        self
    }

    /// Treat `@` as a letter from the start (for .sty/.cls files).
    pub fn with_at_letter(mut self, at_letter: bool) -> Self {
        self.at_letter = at_letter;
        self
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>> {
        let mut tokens = Vec::new();

        while !self.is_at_end() {
            if let Some(token) = self.next_token()? { tokens.push(token) }
            tokens.append(&mut self.pending);
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
        
        if self.math_mode {
            match ch {
                '\\' => return self.lex_command(),
                // comments exist in math too (macro bodies expanded there)
                '%' => {
                    while !self.is_at_end() && self.current_char() != '\n' {
                        self.advance();
                    }
                    if !self.is_at_end() {
                        self.advance();
                    }
                    while !self.is_at_end() && matches!(self.current_char(), ' ' | '\t') {
                        self.advance();
                    }
                    return Ok(None);
                }
                '{' => {
                    self.advance();
                    return Ok(Some(Token::LeftBrace));
                }
                '}' => {
                    self.advance();
                    return Ok(Some(Token::RightBrace));
                }
                '[' => {
                    self.advance();
                    return Ok(Some(Token::LeftBracket));
                }
                ']' => {
                    self.advance();
                    return Ok(Some(Token::RightBracket));
                }
                '&' => {
                    self.advance();
                    return Ok(Some(Token::Ampersand));
                }
                ' ' | '\t' | '\r' | '\n' => {
                    let mut ws = String::new();
                    while !self.is_at_end() && matches!(self.current_char(), ' ' | '\t' | '\r' | '\n') {
                        ws.push(self.current_char());
                        self.advance();
                    }
                    return Ok(Some(Token::Whitespace(ws)));
                }
                _ => {
                    // Each other character is its own token (as in TeX math)
                    self.advance();
                    return Ok(Some(Token::Text(ch.to_string())));
                }
            }
        }

        // expl3 syntax: spaces and line ends are ignored, `~` is a space
        if self.expl_syntax {
            match ch {
                ' ' | '\t' | '\r' | '\n' => {
                    while !self.is_at_end() && matches!(self.current_char(), ' ' | '\t' | '\r' | '\n') {
                        self.advance();
                    }
                    return Ok(None);
                }
                '~' => {
                    self.advance();
                    return Ok(Some(Token::Whitespace(String::from(" "))));
                }
                _ => {}
            }
        }

        // A \MakeShortVerb character: |code| is inline verbatim
        if self.short_verb_chars.contains(&ch) {
            self.advance();
            let mut content = String::new();
            while !self.is_at_end() && self.current_char() != ch && self.current_char() != '\n' {
                content.push(self.current_char());
                self.advance();
            }
            if !self.is_at_end() && self.current_char() == ch {
                self.advance();
            }
            return Ok(Some(Token::Verb(content)));
        }

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

        if self.math_mode && !ch.is_alphabetic() && !(ch == '@' && self.at_letter) {
            // Control symbols are kept verbatim in math: \, \; \! \| \{ \$ ...
            self.advance();
            return Ok(Some(Token::Command(ch.to_string())));
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
            let content = self.read_math_until(&['\\', ')'], true);
            return Ok(Some(Token::InlineMath(content)));
        }
        if ch == ')' {
            self.advance();
            return Ok(None);
        }

        // Escaped literal characters: \% \$ \& \# \_ \{ \}
        // (under expl3 syntax `_` is a letter: `\__module_fn:n`)
        if matches!(ch, '%' | '$' | '&' | '#' | '{' | '}') || (ch == '_' && !self.expl_syntax) {
            self.advance();
            return Ok(Some(Token::Text(ch.to_string())));
        }

        // Accent and spacing control symbols: emitted as single-char commands,
        // resolved by the parser (\' \` \^ \" \~ \= \. accents; \, \; \: \! spacing)
        // (\> \< \+ are tabbing commands; \> is also a math space)
        if matches!(ch, '\'' | '`' | '^' | '"' | '~' | '=' | '.' | ',' | ';' | '!' | '>' | '<' | '+')
            || (ch == ':' && !self.expl_syntax)
        {
            self.advance();
            return Ok(Some(Token::Command(ch.to_string())));
        }

        // Explicit interword space: "\ " or backslash before line end. A
        // control space is a real space: never skipped like ordinary ones
        if ch == ' ' || ch == '\t' || ch == '\n' {
            self.advance();
            return Ok(Some(Token::Text(String::from(" "))));
        }

        // Discretionary hyphen \-, italic correction \/: no output
        if matches!(ch, '-' | '/' | '*') {
            self.advance();
            return Ok(None);
        }
        // The space factor \@ is a token (a macro argument: \makeactive\@)
        if ch == '@' && !self.at_letter {
            self.advance();
            return Ok(Some(Token::Command(String::from("@"))));
        }

        // Lex command name
        let mut cmd_name = self.lex_command_name();
        // expl3 names for TeX primitives: the TeX conditional engine and
        // kernel helpers handle them under their classic names
        if self.expl_syntax && cmd_name.contains(':') {
            if let Some(primitive) = expl_primitive(&cmd_name) {
                cmd_name = primitive.to_string();
            }
        }
        match cmd_name.as_str() {
            "makeatletter" => self.at_letter = true,
            "makeatother" => self.at_letter = false,
            // expl3 packages/classes are read with expl3 syntax on
            "ExplSyntaxOn" | "ProvidesExplPackage" | "ProvidesExplClass" | "ProvidesExplFile" => {
                self.expl_syntax = true
            }
            "ExplSyntaxOff" => self.expl_syntax = false,
            _ => {}
        }

        // \def%{...}, \let%=...: `%` was made active (catcode 13) for this; it
        // is the name being defined, not a comment
        if matches!(cmd_name.as_str(), "def" | "gdef" | "edef" | "xdef" | "let")
            && !self.is_at_end()
            && self.current_char() == '%'
        {
            self.advance();
            self.pending.push(Token::Text(String::from("%")));
            return Ok(Some(Token::Command(cmd_name)));
        }

        if cmd_name.is_empty() {
            // Unknown control symbol: keep the symbol itself as text
            self.advance();
            return Ok(Some(Token::Text(ch.to_string())));
        }

        if self.math_mode {
            // In math only environment names need structure; everything
            // else is a plain control word
            return Ok(Some(match cmd_name.as_str() {
                "begin" | "end" => {
                    self.skip_inline_spaces_before_group();
                    match self.read_simple_group() {
                        Some(name) if cmd_name == "begin" => Token::BeginEnvironment(name),
                        Some(name) => Token::EndEnvironment(name),
                        None => Token::Command(cmd_name),
                    }
                }
                _ => Token::Command(cmd_name),
            }));
        }

        // \verb|...| and friends capture raw content with arbitrary delimiters
        // \MakeShortVerb{\|} / \DeleteShortVerb{\|} (and listings' variants)
        if matches!(cmd_name.as_str(), "MakeShortVerb" | "DefineShortVerb" | "lstMakeShortInline"
            | "DeleteShortVerb" | "UndefineShortVerb" | "lstDeleteShortInline")
        {
            let start = self.position;
            if !self.is_at_end() && self.current_char() == '*' {
                self.advance();
            }
            self.skip_optional_bracket_group();
            let braced = !self.is_at_end() && self.current_char() == '{';
            if braced {
                self.advance();
            }
            if !self.is_at_end() && self.current_char() == '\\' {
                self.advance();
            }
            if !self.is_at_end() {
                let delimiter = self.current_char();
                self.advance();
                if braced && !self.is_at_end() && self.current_char() == '}' {
                    self.advance();
                }
                if !delimiter.is_alphanumeric() && !delimiter.is_whitespace() {
                    if cmd_name.starts_with("Delete") || cmd_name.starts_with("Undefine") || cmd_name.starts_with("lstDelete") {
                        self.short_verb_chars.remove(&delimiter);
                    } else {
                        self.short_verb_chars.insert(delimiter);
                    }
                    return Ok(None);
                }
            }
            self.rewind_to(start);
            return Ok(Some(Token::Command(cmd_name)));
        }

        // (expl3 code is programming: an inline-verbatim command there is
        // part of a definition, read when the definition runs)
        if (matches!(cmd_name.as_str(), "verb" | "lstinline" | "Verb" | "spverb" | "mintinline"
            | "pyth" | "cppinline" | "inlinecode")
            || self.verb_commands.contains(&cmd_name))
            && !self.expl_syntax
        {
            if cmd_name == "mintinline" {
                // \mintinline[opts]{lang}{code} or \mintinline{lang}|code|
                self.skip_optional_bracket_group();
                let _ = self.read_simple_group();
            } else if cmd_name != "verb" {
                // \lstinline[language=C]|code|
                self.skip_optional_bracket_group();
            }
            return self.lex_verb();
        }

        // Commands that define a macro from a verbatim-delimited argument:
        // \verbtocs{\name}|text|, \verbdef\name|text|, \SaveVerb{name}|text|.
        // The text may contain unbalanced braces, so it must be read raw.
        if matches!(cmd_name.as_str(), "verbtocs" | "verbdef" | "SaveVerb" | "Verbdef") {
            let start = self.position;
            self.skip_inline_spaces_before_group();
            let name = if !self.is_at_end() && self.current_char() == '{' {
                self.read_simple_group().map(|n| n.trim().trim_start_matches('\\').to_string())
            } else if !self.is_at_end() && self.current_char() == '\\' {
                self.advance();
                Some(self.lex_command_name())
            } else {
                None
            };
            if let Some(name) = name.filter(|n| !n.is_empty()) {
                while !self.is_at_end() && matches!(self.current_char(), ' ' | '\t') {
                    self.advance();
                }
                if !self.is_at_end() && self.current_char() != '\n' {
                    let delim = self.current_char();
                    self.advance();
                    let mut content = String::new();
                    while !self.is_at_end() && self.current_char() != delim && self.current_char() != '\n' {
                        content.push(self.current_char());
                        self.advance();
                    }
                    if !self.is_at_end() && self.current_char() == delim {
                        self.advance();
                    }
                    self.pending.push(Token::Text(name));
                    self.pending.push(Token::Verb(content));
                    return Ok(Some(Token::Command(String::from("texed@verbdef"))));
                }
            }
            self.rewind_to(start);
            return Ok(Some(Token::Command(cmd_name)));
        }

        // URL arguments are verbatim in LaTeX: % # ~ _ & are literal there
        if matches!(cmd_name.as_str(), "url" | "nolinkurl" | "path" | "burl" | "href" | "doi"
            | "email" | "uri")
        {
            let start = self.position;
            self.skip_inline_spaces_before_group();
            if !self.is_at_end() && self.current_char() == '[' && cmd_name == "href" {
                // \href[options]{url}{text}: leave options to the parser
                self.rewind_to(start);
                return Ok(Some(Token::Command(cmd_name)));
            }
            if let Some(raw) = self.read_verbatim_group() {
                self.pending.push(Token::LeftBrace);
                self.pending.push(Token::Text(raw));
                self.pending.push(Token::RightBrace);
                return Ok(Some(Token::Command(cmd_name)));
            }
            self.rewind_to(start);
            return Ok(Some(Token::Command(cmd_name)));
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
            "section" => self.lex_section(1, &cmd_name),
            "subsection" => self.lex_section(2, &cmd_name),
            "subsubsection" => self.lex_section(3, &cmd_name),
            "chapter" => self.lex_section(0, &cmd_name),
            "part" => self.lex_section(0, &cmd_name),
            "paragraph" => self.lex_section(4, &cmd_name),
            "subparagraph" => self.lex_section(5, &cmd_name),
            _ => Ok(Some(Token::Command(cmd_name))),
        }
    }

    /// Skip a `[...]` group directly at the cursor (verbatim, no nesting).
    fn skip_optional_bracket_group(&mut self) {
        if !self.is_at_end() && self.current_char() == '[' {
            let mut look = self.position + 1;
            while look < self.input.len() && self.input[look] != ']' && self.input[look] != '\n' {
                look += 1;
            }
            if look < self.input.len() && self.input[look] == ']' {
                while self.position <= look {
                    self.advance();
                }
            }
        }
    }

    /// Read a `{...}` group verbatim (balanced braces, no comment or escape
    /// processing, single paragraph). Consumes it on success.
    fn read_verbatim_group(&mut self) -> Option<String> {
        if self.is_at_end() || self.current_char() != '{' {
            return None;
        }
        let mut depth = 0usize;
        let mut look = self.position;
        let mut content = String::new();
        while look < self.input.len() {
            let ch = self.input[look];
            match ch {
                '{' => {
                    depth += 1;
                    if depth > 1 {
                        content.push(ch);
                    }
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        while self.position <= look {
                            self.advance();
                        }
                        return Some(content);
                    }
                    content.push(ch);
                }
                '\n' => {
                    if look + 1 < self.input.len() && self.input[look + 1] == '\n' {
                        return None;
                    }
                }
                // `\%` and `\#` in URLs are escapes for the literal character
                '\\' if look + 1 < self.input.len() && matches!(self.input[look + 1], '%' | '#' | '&' | '_' | '~' | '$') => {
                    content.push(self.input[look + 1]);
                    look += 2;
                    continue;
                }
                _ => content.push(ch),
            }
            look += 1;
        }
        None
    }

    fn lex_verb(&mut self) -> Result<Option<Token>> {
        // Optional star variant (\verb*)
        if !self.is_at_end() && self.current_char() == '*' {
            self.advance();
        }

        // TeX skips the spaces (and line end) after a control word, and
        // expl3 code ignores spaces: the delimiter is the next visible
        // character, but never across a blank line
        let mut newlines = 0;
        while !self.is_at_end() && matches!(self.current_char(), ' ' | '\t' | '\n') {
            if self.current_char() == '\n' {
                newlines += 1;
                if newlines > 1 {
                    break;
                }
            }
            self.advance();
        }

        if self.is_at_end() || self.current_char() == '\n' {
            return Ok(Some(Token::Command(String::from("verb"))));
        }

        let delim = self.current_char();
        self.advance();
        let close = if delim == '{' { '}' } else { delim };

        // \verb cannot span lines: an unterminated \verb ends at the line end.
        // A `{`-delimited body (\mintinline, \lstinline{...}) nests braces.
        let mut content = String::new();
        let mut depth = 0usize;
        while !self.is_at_end() && self.current_char() != '\n' {
            let ch = self.current_char();
            if delim == '{' && ch == '{' {
                depth += 1;
            } else if ch == close {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            content.push(ch);
            self.advance();
        }
        if !self.is_at_end() && self.current_char() == close {
            self.advance(); // consume closing delimiter
        }

        Ok(Some(Token::Verb(content)))
    }

    fn lex_command_name(&mut self) -> String {
        let mut name = String::new();

        while !self.is_at_end() {
            let ch = self.current_char();
            if ch.is_alphabetic()
                || (ch == '@' && self.at_letter)
                || (self.expl_syntax && (ch == '_' || ch == ':'))
            {
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

        self.skip_inline_spaces_before_group();
        let Some(env_name) = self.read_simple_group() else {
            return Ok(Some(Token::Command(String::from("begin"))));
        };
        let env_name = env_name.trim().to_string();

        // Verbatim-like environments capture their body raw so that %, $, \ etc.
        // inside code are not interpreted as LaTeX.
        if is_verbatim_environment(&env_name) || self.verbatim_envs.contains(&env_name) {
            let content = self.read_until_sequence(&format!("\\end{{{}}}", env_name));
            return Ok(Some(Token::VerbatimEnv {
                name: env_name,
                content,
            }));
        }

        if is_math_environment(&env_name) {
            let end = format!("\\end{{{}}}", env_name);
            if let Some(content) = self.try_read_until_sequence(&end) {
                return Ok(Some(Token::MathEnv {
                    name: env_name,
                    content,
                }));
            }
        }

        Ok(Some(Token::BeginEnvironment(env_name)))
    }

    fn lex_end_environment(&mut self) -> Result<Option<Token>> {
        // Don't skip whitespace - let it be tokenized separately
        
        self.skip_inline_spaces_before_group();
        match self.read_simple_group() {
            Some(env_name) => Ok(Some(Token::EndEnvironment(env_name.trim().to_string()))),
            None => Ok(Some(Token::Command(String::from("end")))),
        }
    }

    fn lex_section(&mut self, level: u8, name: &str) -> Result<Option<Token>> {
        let mut flags = SectionFlags {
            starred: false,
            part: name == "part",
        };
        // Starred variant (\section*) — unnumbered, same heading
        if !self.is_at_end() && self.current_char() == '*' {
            self.advance();
            flags.starred = true;
        }
        let start = self.position;
        self.skip_inline_spaces_before_group();
        if !self.is_at_end() && self.current_char() != '{' && self.current_char() != '[' {
            // allow whitespace/newline before the title
            let mut look = self.position;
            while look < self.input.len() && matches!(self.input[look], ' ' | '\t' | '\n') {
                look += 1;
            }
            if look < self.input.len() && matches!(self.input[look], '{' | '[') {
                while self.position < look {
                    self.advance();
                }
            }
        }

        if self.is_at_end() {
            self.rewind_to(start);
            return Ok(Some(Token::Command(name.to_string())));
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
            self.rewind_to(start);
            return Ok(Some(Token::Command(name.to_string())));
        }

        self.advance(); // consume '{'
        let title = self.read_balanced_braces_content();

        Ok(Some(Token::Section(level, title, flags)))
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
        
        self.skip_inline_spaces_before_group();
        match self.read_simple_group() {
            Some(label) => Ok(Some(Token::Label(label.trim().to_string()))),
            None => Ok(Some(Token::Command(String::from("label")))),
        }
    }

    fn lex_ref(&mut self, kind: &str) -> Result<Option<Token>> {
        // Don't skip whitespace - let it be tokenized separately
        
        // Starred variants (\ref*, \cref*) only suppress hyperlinks
        if !self.is_at_end() && self.current_char() == '*' {
            self.advance();
        }
        self.skip_inline_spaces_before_group();
        match self.read_simple_group() {
            Some(reference) => Ok(Some(Token::Ref {
                kind: kind.to_string(),
                label: reference.trim().to_string(),
            })),
            None => Ok(Some(Token::Command(kind.to_string()))),
        }
    }

    fn lex_cite(&mut self, kind: &str) -> Result<Option<Token>> {
        // Don't skip whitespace - let it be tokenized separately
        
        if !self.is_at_end() && self.current_char() == '{' {
            if let Some(citation) = self.read_simple_group() {
                return Ok(Some(Token::Cite {
                    kind: kind.to_string(),
                    keys: citation,
                }));
            }
        }
        Ok(Some(Token::Command(kind.to_string())))
    }

    fn lex_math(&mut self) -> Result<Option<Token>> {
        self.advance(); // consume first '$'

        if !self.is_at_end() && self.current_char() == '$' {
            // Display math $$...$$
            self.advance(); // consume second '$'
            return Ok(Some(match self.read_until_double_dollar() {
                Some(content) => Token::DisplayMath(content),
                None => Token::Text(String::from("$$")),
            }));
        }

        // Inline math $...$ (cannot span a paragraph break in TeX, so an
        // unterminated $ ends at the next blank line instead of eating the
        // rest of the document)
        let (content, end) = self.scan_math(&['$'], true, true);
        if end == MathEnd::NotMath || (end != MathEnd::Closed && content.trim().is_empty()) {
            // Not math: a literal dollar sign
            return Ok(Some(Token::Text(String::from("$"))));
        }
        Ok(Some(Token::InlineMath(content)))
    }

    fn lex_display_math_bracket(&mut self) -> Result<Option<Token>> {
        let content = self.read_math_until(&['\\', ']'], false);
        Ok(Some(Token::DisplayMath(content)))
    }

    fn read_until_double_dollar(&mut self) -> Option<String> {
        match self.scan_math(&['$', '$'], true, true) {
            (_, MathEnd::NotMath) => None,
            (content, _) => Some(content),
        }
    }

    /// Read math content up to (and consuming) the closing delimiter,
    /// skipping escaped characters (`\$`) and `%` comments. If the delimiter
    /// never appears, stop at the first blank line when `stop_at_par` is set
    /// (TeX forbids paragraph breaks in `$...$`/`$$...$$`), otherwise at a
    /// blank line only if the delimiter is missing from the rest of the input.
    fn read_math_until(&mut self, close: &[char], stop_at_par: bool) -> String {
        self.read_math_until_checked(close, stop_at_par).0
    }

    /// Read math content up to (and consuming) the closing delimiter,
    /// treating `\x` as an escaped pair (so `\$` never closes `$...$`) and
    /// dropping `%` comments. Returns the content and whether the delimiter
    /// was found. Unterminated math ends at the first blank line (TeX forbids
    /// paragraph breaks in math), so a stray `$` cannot swallow the rest of
    /// the document; when `stop_at_par` is false the blank line is only used
    /// as the end if the delimiter never appears.
    fn read_math_until_checked(&mut self, close: &[char], stop_at_par: bool) -> (String, bool) {
        let (content, end) = self.scan_math(close, stop_at_par, false);
        (content, end == MathEnd::Closed)
    }

    /// Scan math content (see read_math_until_checked). With
    /// `abort_on_structure`, a `$` that cannot be math — its "content"
    /// would close a group it did not open, or contain `\par` — is reported
    /// as `MathEnd::NotMath` and nothing is consumed: in TeX such a `$` is an
    /// ordinary token (e.g. inside a macro body being defined).
    fn scan_math(&mut self, close: &[char], stop_at_par: bool, abort_on_structure: bool) -> (String, MathEnd) {
        let start = self.position;
        let mut content = String::new();
        let mut par_break_at: Option<(usize, usize)> = None; // (position, content len)
        let mut depth: i64 = 0;

        while !self.is_at_end() {
            if self.matches_sequence(close) {
                for _ in 0..close.len() {
                    self.advance();
                }
                return (content, MathEnd::Closed);
            }
            let ch = self.current_char();
            if ch == '\\' {
                if abort_on_structure && self.matches_sequence(&['\\', 'p', 'a', 'r'])
                    && !self.input.get(self.position + 4).is_some_and(|c| c.is_alphabetic())
                {
                    self.rewind_to(start);
                    return (String::new(), MathEnd::NotMath);
                }
                content.push(ch);
                self.advance();
                if !self.is_at_end() {
                    content.push(self.current_char());
                    self.advance();
                }
                continue;
            }
            if abort_on_structure {
                if ch == '{' {
                    depth += 1;
                } else if ch == '}' {
                    depth -= 1;
                    if depth < 0 {
                        self.rewind_to(start);
                        return (String::new(), MathEnd::NotMath);
                    }
                }
            }
            if ch == '%' {
                while !self.is_at_end() && self.current_char() != '\n' {
                    self.advance();
                }
                continue;
            }
            if ch == '\n' && par_break_at.is_none() {
                let mut look = self.position + 1;
                while look < self.input.len() && matches!(self.input[look], ' ' | '\t' | '\r') {
                    look += 1;
                }
                if look < self.input.len() && self.input[look] == '\n' {
                    if stop_at_par {
                        return (content, MathEnd::Unterminated);
                    }
                    par_break_at = Some((self.position, content.len()));
                }
            }
            content.push(ch);
            self.advance();
        }

        if let Some((pos, len)) = par_break_at {
            self.rewind_to(pos);
            content.truncate(len);
        }
        (content, MathEnd::Unterminated)
    }

    /// Move the cursor back to an earlier position, recomputing line info.
    fn rewind_to(&mut self, position: usize) {
        self.position = position;
        self.line = 1 + self.input[..position].iter().filter(|&&c| c == '\n').count();
        self.column = 1;
    }

    /// Skip spaces/tabs (not newlines-with-blank-lines) between a command
    /// and its argument group: `\begin {itemize}` is valid TeX.
    fn skip_inline_spaces_before_group(&mut self) {
        let mut look = self.position;
        while look < self.input.len() && matches!(self.input[look], ' ' | '\t' | '\r') {
            look += 1;
        }
        if look < self.input.len() && self.input[look] == '{' {
            while self.position < look {
                self.advance();
            }
        }
    }

    /// Read a simple `{...}` group (no nested braces, no paragraph break),
    /// as used for environment names, labels and citation keys. On success
    /// the group is consumed; otherwise nothing is consumed and None is
    /// returned so a stray brace can never swallow the rest of the document.
    fn read_simple_group(&mut self) -> Option<String> {
        if self.is_at_end() || self.current_char() != '{' {
            return None;
        }
        let mut look = self.position + 1;
        let mut content = String::new();
        let mut newlines = 0;
        while look < self.input.len() {
            let ch = self.input[look];
            match ch {
                '}' => {
                    while self.position <= look {
                        self.advance();
                    }
                    return Some(content);
                }
                '{' => return None,
                '\n' => {
                    newlines += 1;
                    if newlines > 1 {
                        return None;
                    }
                    content.push(' ');
                }
                '%' => {
                    // comment inside the group: skip to end of line
                    while look < self.input.len() && self.input[look] != '\n' {
                        look += 1;
                    }
                    continue;
                }
                _ => {
                    newlines = 0;
                    content.push(ch);
                }
            }
            look += 1;
        }
        None
    }

    /// Like read_until_sequence, but only consumes input when the sequence
    /// is actually found.
    fn try_read_until_sequence(&mut self, sequence: &str) -> Option<String> {
        let seq: Vec<char> = sequence.chars().collect();
        let mut look = self.position;
        while look + seq.len() <= self.input.len() {
            if self.input[look..look + seq.len()] == seq[..] {
                let content: String = self.input[self.position..look].iter().collect();
                while self.position < look + seq.len() {
                    self.advance();
                }
                return Some(content);
            }
            look += 1;
        }
        None
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
                // TeX skips the spaces at the start of every line
                while self.position < lookahead {
                    self.advance();
                }
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

/// The classic TeX/LaTeX name of an expl3 primitive or kernel helper.
fn expl_primitive(name: &str) -> Option<&'static str> {
    Some(match name {
        "else:" => "else",
        "fi:" => "fi",
        "or:" => "or",
        "if_true:" => "iftrue",
        "if_false:" => "iffalse",
        "if_meaning:w" => "ifx",
        "if_int_compare:w" => "ifnum",
        "if_int_odd:w" => "ifodd",
        "if_dim:w" => "ifdim",
        "if_case:w" => "ifcase",
        "if_catcode:w" => "ifcat",
        "if_charcode:w" => "if",
        "if:w" => "if",
        "if_mode_math:" => "ifmmode",
        "if_mode_horizontal:" => "ifhmode",
        "if_mode_vertical:" => "ifvmode",
        "if_mode_inner:" => "ifinner",
        "if_cs_exist:N" => "ifdefined",
        "if_eof:w" => "ifeof",
        "if_hbox:N" => "ifhbox",
        "if_vbox:N" => "ifvbox",
        "if_box_empty:N" => "ifvoid",
        "exp_after:wN" => "expandafter",
        "exp_stop_f:" | "scan_stop:" | "prg_do_nothing:" => "relax",
        "use_none:n" => "@gobble",
        "use_none:nn" => "@gobbletwo",
        "use_i:nn" => "@firstoftwo",
        "use_ii:nn" => "@secondoftwo",
        "use_i:nnn" => "@firstofthree",
        "group_begin:" => "begingroup",
        "group_end:" => "endgroup",
        "tex_def:D" => "def",
        "tex_edef:D" => "edef",
        "tex_gdef:D" => "gdef",
        "tex_let:D" => "let",
        "tex_relax:D" => "relax",
        "tex_par:D" => "par",
        "par:" => "par",
        _ => return None,
    })
}

/// Find environments a text defines as verbatim, with their listing
/// language if given: `\lstnewenvironment{code}[...]{\lstset{language=C}}{}`,
/// `\DefineVerbatimEnvironment{code}{Verbatim}{...}`, `\newminted{py}{...}`
/// (-> `pycode`), `\newminted[name]{lang}{...}`, `\newtcblisting{name}`,
/// `\newenvironment{name}{\verbatim}{\endverbatim}`.
pub fn scan_verbatim_env_definitions(text: &str) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    let has_definer = |text: &str| {
        text.contains("lstnewenvironment")
            || text.contains("VerbatimEnvironment")
            || text.contains("newminted")
            || text.contains("tcblisting")
            || text.contains("TCBListing")
            || text.contains("\\verbatim")
            || text.contains("verbatim_start")
    };
    // (cheap test first: this runs for every lexed fragment)
    if !has_definer(text) {
        return out;
    }
    // Definitions inside comments or inside verbatim bodies (a tutorial's
    // example code) do not exist for TeX: scan only live source
    let stripped = strip_comments_and_verbatim(text);
    let text = stripped.as_str();
    if !(text.contains("lstnewenvironment")
        || text.contains("VerbatimEnvironment")
        || text.contains("newminted")
        || text.contains("tcblisting")
        || text.contains("TCBListing")
        || text.contains("\\verbatim")
        || text.contains("verbatim_start")
        || text.contains("VerbatimEnvironment"))
    {
        return out;
    }
    let group_after = |s: &str| -> Option<(String, usize)> {
        let trimmed = s.trim_start();
        let offset = s.len() - trimmed.len();
        let rest = trimmed.strip_prefix('{')?;
        let end = rest.find('}')?;
        Some((rest[..end].trim().to_string(), offset + end + 2))
    };
    let language_of = |s: &str| -> Option<String> {
        let window = &s[..s.len().min(400)];
        let pos = window.find("language")?;
        let after = window[pos + 8..].trim_start().strip_prefix('=')?.trim_start();
        let lang: String = after
            .trim_start_matches('{')
            .chars()
            .take_while(|c| c.is_alphanumeric() || matches!(c, '+' | '#' | '-'))
            .collect();
        (!lang.is_empty()).then(|| lang.to_lowercase())
    };
    for (pattern, kind) in [
        ("\\lstnewenvironment", 0),
        ("\\DefineVerbatimEnvironment", 0),
        ("\\RecustomVerbatimEnvironment", 0),
        ("\\CustomVerbatimEnvironment", 0),
        ("\\newtcblisting", 0),
        ("\\NewTCBListing", 0),
        ("\\DeclareTCBListing", 0),
        ("\\newminted", 1),
        ("\\newenvironment", 2),
        ("\\renewenvironment", 2),
        ("\\NewDocumentEnvironment", 3),
        ("\\RenewDocumentEnvironment", 3),
        ("\\DeclareDocumentEnvironment", 3),
    ] {
        let mut search = 0;
        while let Some(pos) = text[search..].find(pattern) {
            let start = search + pos + pattern.len();
            search = start;
            let rest = &text[start..];
            match kind {
                0 => {
                    let rest = rest.trim_start_matches('*');
                    let rest = if rest.trim_start().starts_with('[') {
                        // \newtcblisting[options]{name}
                        rest.find(']').map(|e| &rest[e + 1..]).unwrap_or(rest)
                    } else {
                        rest
                    };
                    if let Some((name, used)) = group_after(rest) {
                        if !name.is_empty() && !name.contains('\\') {
                            out.push((name, language_of(&rest[used..])));
                        }
                    }
                }
                1 => {
                    // \newminted[envname]{lang}{opts} or \newminted{lang}{opts}
                    let trimmed = rest.trim_start();
                    let (explicit, rest) = if let Some(r) = trimmed.strip_prefix('[') {
                        match r.find(']') {
                            Some(e) => (Some(r[..e].trim().to_string()), &r[e + 1..]),
                            None => (None, trimmed),
                        }
                    } else {
                        (None, trimmed)
                    };
                    if rest.starts_with("file") {
                        continue;
                    }
                    if let Some((lang, _)) = group_after(rest) {
                        if !lang.is_empty() {
                            let name = explicit.unwrap_or_else(|| format!("{}code", lang));
                            out.push((name, Some(lang.to_lowercase())));
                        }
                    }
                }
                _ => {
                    // \newenvironment{name}[..]{begin}{end} (kind 2) or
                    // \NewDocumentEnvironment{name}{spec}{begin}{end} (kind 3)
                    // whose begin code switches to verbatim reading
                    if let Some((name, used)) = group_after(rest) {
                        let mut after = &rest[used..];
                        if kind == 3 {
                            if let Some((_, spec_len)) = group_after(after) {
                                after = &after[spec_len..];
                            }
                        } else {
                            after = after.trim_start_matches(|c: char| c == '[' || c == ']' || c.is_ascii_digit() || c.is_whitespace());
                        }
                        if let Some(begin) = balanced_group(after) {
                            let starts_verbatim = [
                                "\\verbatim", "\\Verbatim", "\\VerbatimEnvironment",
                                "verbatim_start:", "\\begin{verbatim}", "\\begin{Verbatim}",
                                "\\begin{lstlisting}", "\\begin{minted}", "\\lstlisting",
                                "\\comment", "\\begin{comment}",
                            ];
                            if !name.contains('\\') && starts_verbatim.iter().any(|m| begin.contains(m)) {
                                out.push((name, None));
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

/// Find commands a text defines as inline verbatim:
/// `\newmintinline[ltx]{latex}{}` -> `ltx`, `\newmintinline{py}{}` ->
/// `pyinline`, `\newmint[name]{lang}{}` -> `name`, `\newmint{lang}{}` ->
/// `lang`.
pub fn scan_verb_command_definitions(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    if !text.contains("\\newmint") {
        return out;
    }
    let stripped = strip_comments_and_verbatim(text);
    let text = stripped.as_str();
    for (pattern, suffix) in [("\\newmintinline", "inline"), ("\\newmint", "")] {
        let mut search = 0;
        while let Some(pos) = text[search..].find(pattern) {
            let start = search + pos + pattern.len();
            search = start;
            let rest = &text[start..];
            if suffix.is_empty() && (rest.starts_with("inline") || rest.starts_with("ed") || rest.starts_with("file")) {
                continue;
            }
            let trimmed = rest.trim_start();
            let (explicit, rest) = if let Some(r) = trimmed.strip_prefix('[') {
                match r.find(']') {
                    Some(e) => (Some(r[..e].trim().to_string()), &r[e + 1..]),
                    None => (None, trimmed),
                }
            } else {
                (None, trimmed)
            };
            let lang: Option<String> = rest
                .trim_start()
                .strip_prefix('{')
                .and_then(|r| r.find('}').map(|e| r[..e].trim().to_string()));
            if let Some(name) = explicit.or_else(|| lang.map(|l| format!("{}{}", l, suffix))) {
                if !name.is_empty() && name.chars().all(|c| c.is_alphabetic()) {
                    out.push(name);
                }
            }
        }
    }
    out
}

/// Remove `%` comments and the bodies of verbatim-like environments, so a
/// scan for definitions sees only live LaTeX source.
fn strip_comments_and_verbatim(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    // the next verbatim begin, searched again only once passed (linear)
    let mut verbatim = find_verbatim_begin(text);
    while pos < text.len() {
        if let Some((begin, _, _)) = &verbatim {
            if *begin < pos {
                verbatim = find_verbatim_begin(&text[pos..]).map(|(b, s, n)| (b + pos, s + pos, n));
            }
        }
        let comment = find_comment_start(&text[pos..]).map(|c| c + pos);
        match (comment, &verbatim) {
            (Some(c), v) if v.as_ref().is_none_or(|(b, _, _)| c < *b) => {
                out.push_str(&text[pos..c]);
                match text[c..].find('\n') {
                    Some(nl) => pos = c + nl,
                    None => break,
                }
            }
            (_, Some((begin, body_start, name))) => {
                out.push_str(&text[pos..*begin]);
                let end_marker = format!("\\end{{{}}}", name);
                match text[*body_start..].find(&end_marker) {
                    Some(e) => pos = body_start + e + end_marker.len(),
                    None => break,
                }
            }
            _ => {
                out.push_str(&text[pos..]);
                break;
            }
        }
    }
    out
}

/// Byte offset of the first unescaped `%` in the text.
fn find_comment_start(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'%' => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// The first `\begin{X}` of a built-in verbatim-like environment:
/// (offset of `\begin`, offset after it, X).
fn find_verbatim_begin(text: &str) -> Option<(usize, usize, String)> {
    let mut search = 0;
    while let Some(pos) = text[search..].find("\\begin{") {
        let begin = search + pos;
        let name_start = begin + 7;
        let close = text[name_start..].find('}')?;
        let name = &text[name_start..name_start + close];
        if is_verbatim_environment(name) || matches!(name, "example" | "examples" | "code" | "codeblock") {
            return Some((begin, name_start + close + 1, name.to_string()));
        }
        search = name_start;
    }
    None
}

/// The content of a leading `{...}` group (balanced), skipping spaces and
/// `%` comments before it.
fn balanced_group(text: &str) -> Option<&str> {
    let mut start = None;
    let mut depth = 0usize;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '%' if start.is_none() => {
                for (_, c2) in chars.by_ref() {
                    if c2 == '\n' {
                        break;
                    }
                }
            }
            '\\' => {
                chars.next();
            }
            '{' => {
                if start.is_none() {
                    start = Some(i + 1);
                }
                depth += 1;
            }
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(&text[start?..i]);
                }
            }
            c if start.is_none() && !c.is_whitespace() => return None,
            _ => {}
        }
    }
    None
}

/// How a math scan ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathEnd {
    Closed,
    Unterminated,
    NotMath,
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
            | "tikzpicture"
            | "prooftree"
            | "picture"
            | "codehigh"
            | "demohigh"
            | "prooftree*"
            | "bprooftree"
            | "tikzcd"
            | "pgfpicture"
            | "circuitikz"
            | "pspicture"
            | "forest"
            | "asy"
            | "asydef"
            | "mpost"
            | "gnuplot"
            | "dot2tex"
            | "tikzonimage"
            | "sagesilent"
            | "sageblock"
            | "pycode"
            | "luacode"
            | "luacode*"
            | "Verbatim*"
            | "spverbatim"
            | "comment*"
    )
}

/// Display-math environments captured raw by the lexer.
pub fn is_math_environment(name: &str) -> bool {
    matches!(
        name,
        "equation"
            | "equation*"
            | "align"
            | "align*"
            | "gather"
            | "gather*"
            | "multline"
            | "multline*"
            | "flalign"
            | "flalign*"
            | "alignat"
            | "alignat*"
            | "eqnarray"
            | "eqnarray*"
            | "displaymath"
            | "math"
            | "dmath"
            | "dmath*"
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
