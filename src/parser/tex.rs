//! TeX-level programming constructs: conditionals (`\if...\else...\fi`,
//! `\newif`, etoolbox/ifthen branches), `\let`, `\csname`, counter
//! formatting, and the recursion-aware macro expander.
//!
//! None of this attempts to be a full TeX interpreter. The goal is that
//! real-world preambles and macro packages degrade gracefully: the branch a
//! document author intended is chosen whenever it can be determined, and
//! nothing ever loops or explodes when it cannot.

use super::{Inline, Parser};
use crate::error::Result;
use crate::lexer::{Lexer, Token};
use crate::state::{alpha_number, roman_number};

/// Prefix marking a command token that must resolve to its built-in meaning
/// (skipping user macros and aliases); produced by `\let` aliases.
pub(super) const BUILTIN_MARK: char = '\u{1}';

/// Maximum simultaneous nested expansions of the same macro.
const MAX_SAME_MACRO_NESTING: usize = 40;
/// Maximum simultaneous nested macro expansions overall.
const MAX_EXPANSION_NESTING: usize = 400;
/// Baseline per-macro expansion allowance; the effective allowance also
/// scales with document size and with how often the macro occurs in the
/// source, so legitimate heavy use is never cut off while a tail-recursive
/// loop (`\def\a{x\a}`) stops after output proportional to the input.
const PER_MACRO_BASE_BUDGET: usize = 1_000;

/// etoolbox/ifthen-style conditionals taking braced arguments; these are
/// *not* TeX primitive conditionals terminated by `\fi`.
const BRACED_CONDITIONALS: &[&str] = &[
    "ifthenelse", "iftoggle", "nottoggle", "ifbool", "notbool", "ifdef", "ifundef",
    "ifcsdef", "ifcsundef", "ifdefempty", "ifcsempty", "ifstrequal", "ifstrempty",
    "ifblank", "ifnumcomp", "ifdimcomp", "ifnumequal", "ifnumgreater", "ifnumless",
    "ifnumodd", "ifboolexpr", "ifdefmacro", "ifdefstring", "ifcsstring", "ifdefstrequal",
    "ifcsmacro", "ifdefltxprotect", "ifltxcounter", "ifdefcounter", "ifdeflength",
    "ifpackageloaded", "ifclassloaded", "iffileexists", "ifcsvoid", "ifdefvoid",
];

/// Conditionals with a statically known outcome under pdfLaTeX.
fn static_conditional(name: &str) -> Option<bool> {
    Some(match name {
        "iftrue" | "ifpdf" | "ifPDFTeX" | "ifpdftex" | "ifhmode" | "if@twoside" => true,
        "iffalse" | "ifxetex" | "ifXeTeX" | "ifluatex" | "ifLuaTeX" | "ifvtex" | "ifmmode"
        | "ifinner" | "ifvmode" | "ifvoid" | "ifeof" | "ifHy@draft" | "if@draft"
        | "ifdraft" | "ifbeamer" | "ifhtml" | "ifHtml" | "if@restonecol" => false,
        _ => return None,
    })
}

/// pdfTeX/LaTeX primitives and kernel commands that `\ifx\cs\undefined`
/// tests commonly probe for.
fn is_known_defined(name: &str) -> bool {
    matches!(
        name,
        "pdfoutput" | "pdftexversion" | "pdfstrcmp" | "pdfshellescape" | "documentclass"
            | "section" | "usepackage" | "textbf" | "emph" | "LaTeX" | "TeX" | "eTeXversion"
            | "protected" | "unexpanded" | "detokenize" | "numexpr" | "dimexpr" | "ifdefined"
            | "ifcsname" | "pdfTeX" | "relax" | "empty" | "@empty"
    )
}

/// An operand of `\if`: a character, an unexpandable control sequence
/// (all of which compare equal under `\if`), or end of input.
enum IfOperand {
    Char(char),
    Unexpandable,
    End,
}

/// What stopped a skipped conditional branch.
enum BranchEnd {
    Else,
    Or,
    Fi,
    Eof,
}

impl Parser {
    /// Handle TeX-level commands. Returns `Ok(Some(result))` when `cmd` was
    /// handled (the current token has already been consumed), `Ok(None)`
    /// when it is not a TeX-level command.
    pub(super) fn parse_tex_command(&mut self, cmd: &str) -> Result<Option<Option<Inline>>> {
        // Aliases from \let\new\old: re-lex the target so it receives its
        // full meaning (environment tokens, \item, ...)
        if self.state.aliases.contains_key(cmd) {
            return self.expand_alias(cmd);
        }
        self.parse_tex_primitive(cmd)
    }

    /// Resolve a `\let` alias.
    fn expand_alias(&mut self, cmd: &str) -> Result<Option<Option<Inline>>> {
        if let Some(target) = self.state.aliases.get(cmd).cloned() {
            if self.consume_expansion_budget(cmd) {
                // a control-sequence name (letters, `@`, expl3 `_`/`:`) is
                // used as is: re-lexing would split `\c_backslash_str`
                let is_name = !target.is_empty()
                    && target.chars().all(|c| c.is_alphabetic() || matches!(c, '@' | '_' | ':'));
                let tokens = if is_name {
                    vec![Token::Command(target.clone())]
                } else {
                    lex_fragment(&format!("\\{}", target))
                };
                // \let captures the *built-in* meaning: mark plain commands so
                // a later \renewcommand of the original name is bypassed
                let tokens = tokens
                    .into_iter()
                    .map(|t| match t {
                        Token::Command(name) if name == target => {
                            Token::Command(format!("{}{}", BUILTIN_MARK, name))
                        }
                        other => other,
                    })
                    .collect();
                self.push_front(tokens);
            }
            return Ok(Some(None));
        }
        Ok(None)
    }

    /// TeX-level commands (conditionals, kernel helpers, \csname, ...),
    /// without alias resolution.
    pub(super) fn parse_tex_primitive(&mut self, cmd: &str) -> Result<Option<Option<Inline>>> {
        match cmd {
            "expandafter" => {
                self.expand_after(0)?;
                return Ok(Some(None));
            }
            // plain TeX \loop body \if... more \repeat: body, test, and while
            // the test holds, more and again (\repeat is \fi)
            "loop" => {
                let mut body = Vec::new();
                let mut depth = 0usize;
                while !self.is_at_end() {
                    let token = self.current_token().clone();
                    match &token {
                        Token::LeftBrace => depth += 1,
                        Token::RightBrace if depth == 0 => break,
                        Token::RightBrace => depth -= 1,
                        Token::Command(c) if c == "repeat" && depth == 0 => {
                            self.advance();
                            break;
                        }
                        Token::Command(c) if super::is_file_end_marker(c) => break,
                        _ => {}
                    }
                    body.push(token);
                    self.advance();
                }
                let id = self.loop_bodies.len();
                self.loop_bodies.push((body, 0));
                self.push_loop_iteration(id);
                return Ok(Some(None));
            }
            "repeat" => {
                // a stray \repeat closes a conditional, like \fi
                self.cond_depth = self.cond_depth.saturating_sub(1);
                return Ok(Some(None));
            }
            name if name.starts_with("texed@loop:") => {
                if let Ok(id) = name["texed@loop:".len()..].parse::<usize>() {
                    self.push_loop_iteration(id);
                }
                return Ok(Some(None));
            }
            // etoolbox / kernel list loops
            "docsvlist" | "forcsvlist" | "dolistloop" | "forlistloop" | "dolistcsloop" | "forlistcsloop"
            | "@for" | "listadd" | "listgadd" | "listeadd" | "listxadd" | "listcsadd" | "listcsgadd" => {
                self.parse_list_loop(cmd)?;
                return Ok(Some(None));
            }
            "global" | "long" | "outer" | "protected" | "immediate"
            | "noexpand" | "unexpanded" | "ignorespaces" | "makeatletter" | "makeatother"
            | "ExplSyntaxOn" | "ExplSyntaxOff" | "endcsname" => return Ok(Some(None)),
            "let" => {
                self.parse_let()?;
                return Ok(Some(None));
            }
            // LaTeX kernel control helpers (optional-argument idioms)
            "@ifnextchar" | "kernel@ifnextchar" | "new@ifnextchar" | "@ifnextchar@sp" => {
                self.parse_ifnextchar()?;
                return Ok(Some(None));
            }
            "@ifstar" | "@ifstar@sp" => {
                let yes = self.take_branch_tokens();
                let no = self.take_branch_tokens();
                self.skip_whitespace_only();
                let starred = matches!(self.current_token(), Token::Text(t) if t.starts_with('*'));
                if starred {
                    self.consume_text_star_prefix();
                }
                self.push_front(if starred { yes } else { no });
                return Ok(Some(None));
            }
            "@ifundefined" | "@ifpackageloaded" | "@ifclassloaded" | "@ifpackagewith"
            | "@ifclasswith" | "IfFileExists" | "InputIfFileExists" | "@ifdefinable"
            | "IfPackageLoadedTF" | "IfClassLoadedTF" | "IfFormatAtLeastTF"
            | "@ifl@t@r" | "IfPackageAtLeastTF" => {
                self.skip_whitespace_and_comments();
                let subject = self.parse_braced_text()?;
                if matches!(cmd, "@ifpackagewith" | "@ifclasswith" | "@ifl@t@r" | "IfPackageAtLeastTF") {
                    self.skip_whitespace_and_comments();
                    let _ = self.parse_braced_text()?;
                }
                let yes = self.take_branch_tokens();
                let no = self.take_branch_tokens();
                let value = match cmd {
                    "@ifundefined" => !self.is_defined_command(subject.trim().trim_start_matches('\\')),
                    "@ifdefinable" => true,
                    // a local file is looked up; a bare name not found here
                    // is assumed to be an installed package file
                    "IfFileExists" | "InputIfFileExists" => {
                        let name = self.expand_parameterless_macros(&subject);
                        let name = name.trim();
                        self.local_file_exists(name)
                            || (cmd == "IfFileExists" && !name.contains('/') && !subject.contains('\\'))
                    }
                    // We behave as a current LaTeX with the usual packages
                    _ => true,
                };
                if cmd == "InputIfFileExists" && !value {
                    self.push_front(no);
                    return Ok(Some(None));
                }
                if cmd == "InputIfFileExists" {
                    // \InputIfFileExists{file}{before}{else}: run `before`;
                    // the file itself is resolved like \input
                    let mut tokens = vec![Token::Command(String::from("input")), Token::LeftBrace, Token::Text(subject), Token::RightBrace];
                    tokens.splice(0..0, yes);
                    self.push_front(tokens);
                    return Ok(Some(None));
                }
                self.push_front(if value { yes } else { no });
                return Ok(Some(None));
            }
            "@firstoftwo" | "@secondoftwo" | "@firstofone" | "@gobble" | "@gobbletwo"
            | "@gobblefour" | "use_i:nn" | "use_ii:nn" | "use:n" | "use_none:n"
            | "use_none:nn" | "@iden" => {
                let (keep, take) = match cmd {
                    "@firstoftwo" | "use_i:nn" => (Some(0), 2),
                    "@secondoftwo" | "use_ii:nn" => (Some(1), 2),
                    "@firstofone" | "use:n" | "@iden" => (Some(0), 1),
                    "@gobble" | "use_none:n" => (None, 1),
                    "@gobbletwo" | "use_none:nn" => (None, 2),
                    _ => (None, 4),
                };
                let mut groups = Vec::new();
                for _ in 0..take {
                    groups.push(self.take_branch_tokens());
                }
                if let Some(index) = keep {
                    let chosen = std::mem::take(&mut groups[index]);
                    self.push_front(chosen);
                }
                return Ok(Some(None));
            }
            "@empty" | "@nil" | "@nnil" | "@sptoken" => return Ok(Some(None)),
            // xparse value tests (arguments arrive as -NoValue- / \BooleanTrue)
            "IfNoValueTF" | "IfNoValueT" | "IfNoValueF" | "IfValueTF" | "IfValueT"
            | "IfValueF" | "IfBooleanTF" | "IfBooleanT" | "IfBooleanF" => {
                self.skip_whitespace_only();
                let arg = self.parse_braced_text()?;
                let arg = arg.trim();
                let condition = if cmd.starts_with("IfBoolean") {
                    arg == "\\BooleanTrue"
                } else if cmd.starts_with("IfNoValue") {
                    arg == "-NoValue-"
                } else {
                    arg != "-NoValue-"
                };
                let (yes, no) = if cmd.ends_with("TF") {
                    (self.take_branch_tokens(), self.take_branch_tokens())
                } else if cmd.ends_with('T') {
                    (self.take_branch_tokens(), Vec::new())
                } else {
                    (Vec::new(), self.take_branch_tokens())
                };
                self.push_front(if condition { yes } else { no });
                return Ok(Some(None));
            }
            "BooleanTrue" | "BooleanFalse" | "NoValue" => return Ok(Some(None)),
            "@ifpackagelater" | "@ifclasslater" => {
                self.skip_whitespace_and_comments();
                let _ = self.parse_braced_text()?;
                self.skip_whitespace_and_comments();
                let _ = self.parse_braced_text()?;
                let yes = self.take_branch_tokens();
                let _no = self.take_branch_tokens();
                self.push_front(yes);
                return Ok(Some(None));
            }
            "texed@verbdef" => {
                // name + verbatim text, from \verbtocs and friends
                if let Token::Text(name) = self.current_token().clone() {
                    self.advance();
                    if let Token::Verb(text) = self.current_token().clone() {
                        self.advance();
                        self.state.macros.insert(
                            name,
                            crate::state::MacroDef {
                                body: text,
                                literal: true,
                                ..Default::default()
                            },
                        );
                    }
                }
                return Ok(Some(None));
            }
            "texed@popdir" => {
                if let Some((_, path)) = self.input_stack.pop() {
                    self.state.unmark_file_included(&path);
                }
                return Ok(Some(None));
            }
            "texed@endpackage" => {
                if let Some(was_preamble) = self.package_preamble_restore.pop() {
                    self.in_preamble = was_preamble;
                }
                return Ok(Some(None));
            }
            // Deferred hooks run immediately: their bodies are usually
            // definitions the document relies on
            "AtBeginDocument" | "AtEndPreamble" | "AtEndOfPackage" | "AtEndOfClass"
            | "AfterEndPreamble" => {
                self.skip_whitespace_and_comments();
                let body = self.collect_braced_tokens();
                self.push_front(body);
                return Ok(Some(None));
            }
            "futurelet" => {
                for _ in 0..3 {
                    self.skip_whitespace_and_comments();
                    self.advance();
                }
                return Ok(Some(None));
            }
            // etoolbox \csuse{name} = \name
            "csuse" => {
                self.skip_whitespace_and_comments();
                let raw = self.parse_braced_text()?;
                // the name may itself be a macro: \csuse{\ol@sectioncs}
                let name = self.expand_parameterless_macros(raw.trim());
                let name = name.trim().trim_start_matches('\\').to_string();
                if !name.is_empty() {
                    self.push_front(vec![Token::Command(name)]);
                }
                return Ok(Some(None));
            }
            "csname" => {
                let name = self.collect_csname();
                if !name.is_empty() {
                    self.push_front(vec![Token::Command(name)]);
                }
                return Ok(Some(None));
            }
            "string" | "detokenize" => {
                self.skip_whitespace_and_comments();
                let text = if matches!(self.current_token(), Token::LeftBrace) {
                    self.parse_braced_text()?
                } else {
                    let token = self.current_token().clone();
                    self.advance();
                    self.token_to_string(&token)
                };
                return Ok(Some(Some(Inline::Text(text))));
            }
            "newif" => {
                self.skip_whitespace_and_comments();
                if let Token::Command(name) = self.current_token().clone() {
                    self.advance();
                    if let Some(base) = name.strip_prefix("if") {
                        self.state.conditionals.insert(base.to_string(), false);
                    }
                }
                return Ok(Some(None));
            }
            // etoolbox booleans
            "newbool" | "providebool" | "booltrue" | "boolfalse" => {
                self.skip_whitespace_only();
                let name = self.parse_braced_text()?.trim().to_string();
                let value = cmd == "booltrue";
                if cmd.ends_with("bool") {
                    self.state.conditionals.entry(name).or_insert(false);
                } else {
                    self.state.conditionals.insert(name, value);
                }
                return Ok(Some(None));
            }
            "setbool" => {
                self.skip_whitespace_only();
                let name = self.parse_braced_text()?.trim().to_string();
                self.skip_whitespace_only();
                let value = self.parse_braced_text()?.trim().eq_ignore_ascii_case("true");
                self.state.conditionals.insert(name, value);
                return Ok(Some(None));
            }
            "newboolean" | "provideboolean" => {
                self.skip_whitespace_and_comments();
                let name = self.parse_braced_text()?.trim().to_string();
                self.state.conditionals.entry(name).or_insert(false);
                return Ok(Some(None));
            }
            "setboolean" => {
                self.skip_whitespace_and_comments();
                let name = self.parse_braced_text()?.trim().to_string();
                self.skip_whitespace_and_comments();
                let value = self.parse_braced_text()?.trim().eq_ignore_ascii_case("true");
                self.state.conditionals.insert(name, value);
                return Ok(Some(None));
            }
            "providetoggle" | "newtoggle" => {
                self.skip_whitespace_and_comments();
                let name = self.parse_braced_text()?.trim().to_string();
                self.state.toggles.entry(name).or_insert(false);
                return Ok(Some(None));
            }
            "settoggle" => {
                self.skip_whitespace_and_comments();
                let name = self.parse_braced_text()?.trim().to_string();
                self.skip_whitespace_and_comments();
                let value = self.parse_braced_text()?.trim().eq_ignore_ascii_case("true");
                self.state.toggles.insert(name, value);
                return Ok(Some(None));
            }
            "toggletrue" | "togglefalse" => {
                self.skip_whitespace_and_comments();
                let name = self.parse_braced_text()?.trim().to_string();
                self.state.toggles.insert(name, cmd == "toggletrue");
                return Ok(Some(None));
            }
            "else" | "or" => {
                // Reached the end of the branch being taken: skip the rest
                // of the conditional.
                if self.cond_depth > 0 {
                    if let BranchEnd::Fi = self.skip_branch(false, false) {
                        self.cond_depth -= 1;
                    }
                }
                return Ok(Some(None));
            }
            "fi" => {
                self.cond_depth = self.cond_depth.saturating_sub(1);
                return Ok(Some(None));
            }
            "the" | "number" | "arabic" | "roman" | "Roman" | "alph" | "Alph" | "value"
            | "romannumeral" | "fnsymbol" => {
                return Ok(Some(self.parse_counter_format(cmd)?));
            }
            _ => {}
        }

        // \fooTrue/\foofalse for \newif booleans
        for (suffix, value) in [("true", true), ("false", false)] {
            if let Some(base) = cmd.strip_suffix(suffix) {
                if self.state.conditionals.contains_key(base) {
                    self.state.conditionals.insert(base.to_string(), value);
                    return Ok(Some(None));
                }
            }
        }

        // \thesection, \thechapter, \theequation, ...
        if let Some(counter) = cmd.strip_prefix("the") {
            if let Some(text) = self.format_named_counter(counter, "arabic") {
                return Ok(Some(Some(Inline::Text(text))));
            }
        }

        if BRACED_CONDITIONALS.contains(&cmd) {
            self.parse_braced_conditional(cmd)?;
            return Ok(Some(None));
        }

        if self.is_tex_conditional(cmd) {
            self.parse_tex_conditional(cmd)?;
            return Ok(Some(None));
        }

        Ok(None)
    }

    /// Is `name` a primitive-style conditional (terminated by `\fi`)?
    pub(super) fn is_tex_conditional(&self, name: &str) -> bool {
        if name == "if" {
            return !self.state.macros.contains_key(name);
        }
        if name.len() <= 2 || !name.starts_with("if") || name == "iff" {
            return false;
        }
        if BRACED_CONDITIONALS.contains(&name) || self.state.macros.contains_key(name) {
            return false;
        }
        true
    }

    /// Evaluate a `\if...` conditional and position the stream at the
    /// chosen branch.
    fn parse_tex_conditional(&mut self, cmd: &str) -> Result<()> {
        if cmd == "ifcase" {
            let n = self.read_tex_number().unwrap_or(0);
            let mut case = 0;
            while case < n {
                match self.skip_branch(true, true) {
                    BranchEnd::Or => case += 1,
                    BranchEnd::Else => break,
                    BranchEnd::Fi | BranchEnd::Eof => return Ok(()),
                }
            }
            self.cond_depth += 1;
            return Ok(());
        }

        let value = self.evaluate_conditional(cmd)?.unwrap_or(true);
        if value {
            self.cond_depth += 1;
        } else {
            match self.skip_branch(true, false) {
                BranchEnd::Else => self.cond_depth += 1,
                BranchEnd::Fi | BranchEnd::Or | BranchEnd::Eof => {}
            }
        }
        Ok(())
    }

    /// `\@ifnextchar<c>{yes}{no}`: peek at the next non-space token.
    fn parse_ifnextchar(&mut self) -> Result<()> {
        self.skip_whitespace_only();
        // The test character: a single token (often `[`, `(`, `*`, `\bgroup`)
        let test = self.current_token().clone();
        self.advance();
        let yes = self.take_branch_tokens();
        let no = self.take_branch_tokens();
        // \@ifnextchar skips spaces before peeking
        while matches!(self.current_token(), Token::Whitespace(_) | Token::Newline) {
            self.advance();
        }
        let next = self.current_token();
        let matches = match (&test, next) {
            (Token::LeftBracket, Token::LeftBracket) => true,
            (Token::RightBracket, Token::RightBracket) => true,
            (Token::Command(c), Token::LeftBrace) if c == "bgroup" => true,
            (Token::Command(c), Token::RightBrace) if c == "egroup" => true,
            (Token::Command(a), Token::Command(b)) => a == b,
            (Token::Text(a), Token::Text(b)) => a.chars().next() == b.chars().next(),
            (Token::Tilde, Token::Tilde) | (Token::Ampersand, Token::Ampersand) => true,
            (Token::Backslash, Token::Backslash) => true,
            _ => false,
        };
        self.push_front(if matches { yes } else { no });
        Ok(())
    }

    /// One branch argument: a braced group's tokens, or a single token.
    pub(super) fn take_branch_tokens(&mut self) -> Vec<Token> {
        // TeX skips spaces and a line end before an undelimited argument
        self.skip_whitespace_only();
        if matches!(self.current_token(), Token::LeftBrace) {
            self.collect_braced_tokens()
        } else if self.is_at_end() {
            Vec::new()
        } else {
            let token = self.current_token().clone();
            self.advance();
            vec![token]
        }
    }

    /// Consume a leading `*` from the current text token.
    fn consume_text_star_prefix(&mut self) {
        if let Token::Text(t) = self.current_token().clone() {
            if let Some(rest) = t.strip_prefix('*') {
                self.advance();
                if !rest.is_empty() {
                    self.push_front(vec![Token::Text(rest.to_string())]);
                }
            }
        }
    }

    /// Determine a conditional's truth value, consuming its operands.
    /// `None` means it cannot be decided statically.
    fn evaluate_conditional(&mut self, cmd: &str) -> Result<Option<bool>> {
        if cmd == "ifmmode" {
            return Ok(Some(self.in_math_expansion));
        }
        // \newif booleans first: a document's own \ifdraft wins over defaults
        if let Some(value) = cmd
            .strip_prefix("if")
            .and_then(|base| self.state.conditionals.get(base))
        {
            return Ok(Some(*value));
        }
        if let Some(value) = static_conditional(cmd) {
            return Ok(Some(value));
        }
        match cmd {
            "ifdefined" => {
                self.skip_whitespace_and_comments();
                let token = self.current_token().clone();
                self.advance();
                Ok(match token {
                    Token::Command(name) => Some(self.is_defined_command(&name)),
                    _ => None,
                })
            }
            "ifcsname" => {
                let name = self.collect_csname();
                Ok(Some(self.is_defined_command(&name)))
            }
            "ifx" => {
                let a = self.next_significant_token();
                let b = self.next_significant_token();
                Ok(self.compare_ifx(a, b))
            }
            "if" | "ifcat" => {
                let a = self.read_if_operand(0);
                let b = self.read_if_operand(0);
                Ok(Some(match (a, b) {
                    (IfOperand::Char(x), IfOperand::Char(y)) => {
                        if cmd == "if" {
                            x == y
                        } else {
                            x.is_alphabetic() == y.is_alphabetic()
                        }
                    }
                    (IfOperand::Unexpandable, IfOperand::Unexpandable) => true,
                    (IfOperand::End, _) | (_, IfOperand::End) => return Ok(None),
                    _ => false,
                }))
            }
            "ifnum" | "ifdim" => Ok(self.read_comparison()),
            "ifodd" => Ok(self.read_tex_number().map(|n| n % 2 == 1)),
            _ => {
                if !cmd.starts_with("if@") && !self.state.conditionals.is_empty() {
                    // Unknown conditional: take the first branch
                }
                Ok(None)
            }
        }
    }

    /// Read one operand of `\if`/`\ifcat`, expanding as TeX does:
    /// `\detokenize{}`/`\string` produce characters, macros expand, and an
    /// expansion that produces nothing makes `\if` read further.
    fn read_if_operand(&mut self, depth: usize) -> IfOperand {
        if depth > 16 {
            return IfOperand::End;
        }
        self.skip_whitespace_and_comments();
        if self.is_at_end() {
            return IfOperand::End;
        }
        let token = self.current_token().clone();
        self.advance();
        match token {
            Token::Text(t) => {
                let mut chars = t.chars();
                let first = chars.next();
                let rest: String = chars.collect();
                if !rest.is_empty() {
                    self.push_front(vec![Token::Text(rest)]);
                }
                first.map_or(IfOperand::End, IfOperand::Char)
            }
            Token::Command(name) if matches!(name.as_str(), "detokenize" | "unexpanded") => {
                self.skip_whitespace_and_comments();
                let text = self.parse_braced_text().unwrap_or_default();
                let text = text.trim().to_string();
                if text.is_empty() {
                    self.read_if_operand(depth + 1)
                } else {
                    let mut chars = text.chars();
                    let first = chars.next().unwrap();
                    let rest: String = chars.collect();
                    if !rest.is_empty() {
                        self.push_front(vec![Token::Text(rest)]);
                    }
                    IfOperand::Char(first)
                }
            }
            Token::Command(name) if name == "string" => {
                let next = self.current_token().clone();
                self.advance();
                match next {
                    Token::Command(_) => IfOperand::Char('\\'),
                    Token::Text(t) => t.chars().next().map_or(IfOperand::End, IfOperand::Char),
                    _ => IfOperand::Unexpandable,
                }
            }
            Token::Command(name) if self.state.macros.contains_key(&name) => {
                let args = self.parse_macro_arguments(&name).unwrap_or_default();
                let body = self
                    .state
                    .macros
                    .get(&name)
                    .map(|d| crate::state::substitute_params(&d.body, &args))
                    .unwrap_or_default();
                if self.consume_expansion_budget(&name) {
                    self.push_front(lex_fragment(&body));
                    self.read_if_operand(depth + 1)
                } else {
                    IfOperand::Unexpandable
                }
            }
            // \the / \number of a register expand to its digits
            Token::Command(name) if matches!(name.as_str(), "the" | "number" | "arabic" | "value") => {
                self.skip_whitespace_only();
                let value = match self.current_token().clone() {
                    Token::Command(register) if matches!(name.as_str(), "the" | "number") => {
                        self.advance();
                        self.command_numeric_value(&register)
                    }
                    _ => self.command_numeric_value(&name),
                };
                match value {
                    Some(value) => {
                        self.push_front(vec![Token::Text(value.to_string())]);
                        self.read_if_operand(depth + 1)
                    }
                    None => IfOperand::Unexpandable,
                }
            }
            Token::LeftBrace => IfOperand::Char('{'),
            Token::RightBrace => IfOperand::Char('}'),
            Token::LeftBracket => IfOperand::Char('['),
            Token::RightBracket => IfOperand::Char(']'),
            Token::Ampersand => IfOperand::Char('&'),
            Token::Tilde => IfOperand::Unexpandable,
            _ => IfOperand::Unexpandable,
        }
    }

    fn is_defined_command(&self, name: &str) -> bool {
        self.state.macros.contains_key(name)
            || self.state.aliases.contains_key(name)
            || self.macro_processor.is_defined(&self.state, name)
            || self.state.environments.contains_key(name)
            || is_known_defined(name)
    }

    fn compare_ifx(&self, a: Option<Token>, b: Option<Token>) -> Option<bool> {
        let (a, b) = (a?, b?);
        let undefined = |name: &str| matches!(name, "undefined" | "@undefined" | "UnDeFiNeD");
        match (&a, &b) {
            (Token::Command(x), Token::Command(y)) if undefined(y) => {
                Some(!self.is_defined_command(x))
            }
            (Token::Command(x), Token::Command(y)) if undefined(x) => {
                Some(!self.is_defined_command(y))
            }
            (Token::Command(x), Token::Command(y)) => {
                if x == y {
                    return Some(true);
                }
                match (self.state.macros.get(x), self.state.macros.get(y)) {
                    (Some(dx), Some(dy)) => {
                        Some(dx.body == dy.body && dx.num_params == dy.num_params)
                    }
                    _ => None,
                }
            }
            _ => Some(a == b),
        }
    }

    /// Next token that is not whitespace or a comment, consumed.
    fn next_significant_token(&mut self) -> Option<Token> {
        self.skip_whitespace_and_comments();
        if self.is_at_end() {
            return None;
        }
        let token = self.current_token().clone();
        self.advance();
        Some(token)
    }

    /// Parse `<number> <relation> <number>` after \ifnum/\ifdim.
    fn read_comparison(&mut self) -> Option<bool> {
        let mut text = String::new();
        let mut relation: Option<char> = None;
        let mut rhs = String::new();
        for _ in 0..12 {
            self.skip_whitespace_and_comments();
            // Math mode splits text per character: merge the run first
            if matches!(self.current_token(), Token::Text(_)) {
                let run = self.take_text_run();
                self.push_front(vec![Token::Text(run)]);
            }
            match self.current_token().clone() {
                Token::Text(t) if relation.is_none() => {
                    if let Some(pos) = t.find(['<', '=', '>']) {
                        text.push_str(&t[..pos]);
                        relation = t[pos..].chars().next();
                        let rest = &t[pos + 1..];
                        self.advance();
                        if !rest.is_empty() {
                            rhs.push_str(rest);
                            break;
                        }
                    } else {
                        text.push_str(&t);
                        self.advance();
                    }
                }
                Token::Text(t) => {
                    // the right-hand number (and unit); anything after it
                    // belongs to the conditional's body
                    self.advance();
                    let (value, rest) = crate::parser::split_tex_value_pub(&t);
                    rhs.push_str(&value);
                    if !rest.is_empty() {
                        self.push_front(vec![Token::Text(rest)]);
                    }
                    break;
                }
                Token::Command(name) => {
                    self.advance();
                    let value = self.command_numeric_value(&name);
                    let target = if relation.is_none() { &mut text } else { &mut rhs };
                    target.push_str(&value.map(|v| v.to_string()).unwrap_or_else(|| "?".into()));
                    if relation.is_some() {
                        break;
                    }
                }
                _ => break,
            }
        }
        let relation = relation?;
        let lhs = parse_tex_dimen(&text)?;
        let rhs = parse_tex_dimen(&rhs)?;
        Some(match relation {
            '<' => lhs < rhs,
            '>' => lhs > rhs,
            _ => (lhs - rhs).abs() < f64::EPSILON,
        })
    }

    /// Numeric value of `\value{x}`, `\c@x` or a counter-like command.
    fn command_numeric_value(&mut self, name: &str) -> Option<i64> {
        if matches!(name, "value" | "arabic") {
            self.skip_whitespace_and_comments();
            let counter = self.parse_braced_text().ok()?;
            return Some(self.counter_value(counter.trim()) as i64);
        }
        if let Some(counter) = name.strip_prefix("c@") {
            return Some(self.counter_value(counter) as i64);
        }
        if let Some(value) = self.count_registers.get(name) {
            return Some(*value);
        }
        self.state
            .macros
            .get(name)
            .and_then(|def| def.body.trim().parse::<i64>().ok())
    }

    /// Read a TeX number (literal digits, `\value{}`, or a numeric macro).
    pub(super) fn read_tex_number(&mut self) -> Option<i64> {
        self.skip_whitespace_and_comments();
        match self.current_token().clone() {
            Token::Text(t) => {
                let digits: String = t
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '-')
                    .collect();
                let rest = t[digits.len()..].to_string();
                self.advance();
                if !rest.is_empty() {
                    self.push_front(vec![Token::Text(rest)]);
                }
                digits.parse().ok()
            }
            Token::Command(name) => {
                self.advance();
                self.command_numeric_value(&name)
            }
            _ => None,
        }
    }

    /// Skip tokens of a false conditional branch up to the matching
    /// `\else`/`\or` (when requested) or `\fi`, which is consumed.
    fn skip_branch(&mut self, stop_at_else: bool, stop_at_or: bool) -> BranchEnd {
        let mut depth = 0usize;
        while !self.is_at_end() {
            // a file ends a skipped conditional, as in TeX ("Incomplete \if;
            // all text was ignored after line n")
            if matches!(self.current_token(), Token::Command(c) if c.starts_with("texed@end") || c == "texed@popdir") {
                return BranchEnd::Fi;
            }
            let token = self.current_token().clone();
            self.advance();
            if let Token::Command(name) = token {
                match name.as_str() {
                    // (\repeat is \let to \fi)
                    "fi" | "repeat" => {
                        if depth == 0 {
                            return BranchEnd::Fi;
                        }
                        depth -= 1;
                    }
                    "else" if depth == 0 && stop_at_else => return BranchEnd::Else,
                    "or" if depth == 0 && stop_at_or => return BranchEnd::Or,
                    // The \ifname after \newif is a name, not a conditional
                    "newif" | "let" | "def" | "gdef" | "edef" | "xdef" => {
                        if matches!(self.current_token(), Token::Command(_)) {
                            self.advance();
                        }
                    }
                    _ if self.is_tex_conditional(&name) => depth += 1,
                    _ => {}
                }
            }
        }
        BranchEnd::Eof
    }

    /// etoolbox / ifthen conditionals: `\iftoggle{x}{yes}{no}` and friends.
    fn parse_braced_conditional(&mut self, cmd: &str) -> Result<()> {
        let take_braced = |parser: &mut Parser| -> Vec<Token> {
            parser.skip_whitespace_and_comments();
            parser.collect_braced_tokens()
        };
        let operand_count = match cmd {
            "ifstrequal" | "ifnumcomp" | "ifdimcomp" | "ifnumequal" | "ifnumgreater"
            | "ifnumless" | "ifdefstring" | "ifcsstring" | "ifdefstrequal" => 2,
            _ => 1,
        };
        let operands: Vec<String> = (0..operand_count)
            .map(|_| {
                let tokens = take_braced(self);
                tokens.iter().map(|t| self.token_to_string(t)).collect::<String>()
            })
            .collect();
        // \ifnumcomp{a}{<}{b}: the relation is a separate argument
        let relation = if matches!(cmd, "ifnumcomp" | "ifdimcomp") {
            let tokens = take_braced(self);
            Some(tokens.iter().map(|t| self.token_to_string(t)).collect::<String>())
        } else {
            None
        };
        let yes = take_braced(self);
        let no = take_braced(self);

        let operand = operands.first().map(|s| s.trim().to_string()).unwrap_or_default();
        let defined = |p: &Parser, s: &str| p.is_defined_command(s.trim().trim_start_matches('\\'));
        let value = match cmd {
            "ifthenelse" => self.evaluate_ifthen(&operand),
            "iftoggle" => self.state.toggles.get(&operand).copied().unwrap_or(false),
            "nottoggle" => !self.state.toggles.get(&operand).copied().unwrap_or(false),
            "ifbool" => self.state.conditionals.get(&operand).copied().unwrap_or(false),
            "notbool" => !self.state.conditionals.get(&operand).copied().unwrap_or(false),
            "ifdef" | "ifcsdef" | "ifdefmacro" | "ifcsmacro" => defined(self, &operand),
            "ifundef" | "ifcsundef" => !defined(self, &operand),
            "ifdefempty" | "ifcsempty" | "ifdefvoid" | "ifcsvoid" => self
                .state
                .macros
                .get(operand.trim_start_matches('\\'))
                .is_none_or(|d| d.body.trim().is_empty()),
            "ifstrequal" | "ifdefstring" | "ifcsstring" | "ifdefstrequal" => {
                operands[0].trim() == operands[1].trim()
            }
            "ifstrempty" | "ifblank" => operand.is_empty(),
            "ifnumequal" => num(&operands[0]) == num(&operands[1]),
            "ifnumgreater" => num(&operands[0]) > num(&operands[1]),
            "ifnumless" => num(&operands[0]) < num(&operands[1]),
            "ifnumodd" => num(&operand).is_some_and(|n| n % 2 != 0),
            "ifnumcomp" | "ifdimcomp" => {
                let (a, b) = (num(&operands[0]), num(&operands[1]));
                match relation.as_deref().map(str::trim) {
                    Some("<") => a < b,
                    Some(">") => a > b,
                    _ => a == b,
                }
            }
            "ifpackageloaded" | "ifclassloaded" | "iffileexists" => true,
            _ => true,
        };
        self.push_front(if value { yes } else { no });
        Ok(())
    }

    /// Evaluate an `\ifthenelse` condition (ifthen package syntax).
    fn evaluate_ifthen(&self, condition: &str) -> bool {
        let condition = condition.trim();
        if let Some(rest) = condition.strip_prefix("\\not") {
            return !self.evaluate_ifthen(rest);
        }
        if let Some(inner) = braced_arg(condition, "\\boolean") {
            return self.state.conditionals.get(inner.trim()).copied().unwrap_or(false);
        }
        if let Some(rest) = condition.strip_prefix("\\equal") {
            let (a, rest) = split_braced(rest);
            let (b, _) = split_braced(rest);
            return a.trim() == b.trim();
        }
        if let Some(inner) = braced_arg(condition, "\\isundefined") {
            return !self.is_defined_command(inner.trim().trim_start_matches('\\'));
        }
        if let Some(pos) = condition.find(['<', '=', '>']) {
            let (a, b) = (num(&condition[..pos]), num(&condition[pos + 1..]));
            return match &condition[pos..pos + 1] {
                "<" => a < b,
                ">" => a > b,
                _ => a == b,
            };
        }
        true
    }

    /// `\let\new\old` / `\let\new=\old`.
    fn parse_let(&mut self) -> Result<()> {
        self.skip_whitespace_and_comments();
        let target = match self.current_token().clone() {
            // \let\csname name\endcsname ...
            Token::Command(name) if name == "csname" => {
                self.advance();
                self.collect_csname()
            }
            Token::Command(name) => {
                self.advance();
                name
            }
            // an active character (`\let =\ ` after \obeyspaces, `\let~=\x`):
            // consume `= value`, nothing to record
            Token::Whitespace(_) | Token::Tilde | Token::Text(_) => {
                self.advance();
                self.skip_whitespace_and_comments();
                if let Token::Text(t) = self.current_token().clone() {
                    if let Some(rest) = t.strip_prefix('=') {
                        self.advance();
                        if !rest.is_empty() {
                            self.push_front(vec![Token::Text(rest.to_string())]);
                        }
                    }
                }
                if matches!(self.current_token(), Token::Whitespace(_)) {
                    self.advance();
                }
                if !self.is_at_end() {
                    self.advance();
                }
                return Ok(());
            }
            _ => return Ok(()),
        };
        if target.is_empty() {
            return Ok(());
        }
        self.skip_whitespace_and_comments();
        if let Token::Text(t) = self.current_token().clone() {
            if let Some(rest) = t.strip_prefix('=') {
                self.advance();
                if !rest.is_empty() {
                    self.push_front(vec![Token::Text(rest.to_string())]);
                }
            }
        }
        // One optional space after '='
        if matches!(self.current_token(), Token::Whitespace(_)) {
            self.advance();
        }
        let source = match self.current_token().clone() {
            Token::Command(name) if name == "csname" => {
                self.advance();
                let name = self.collect_csname();
                // an undefined \csname is \relax
                Token::Command(if name.is_empty() { String::from("relax") } else { name })
            }
            other => {
                self.advance();
                other
            }
        };

        // a built-in logo keeps its text (\let\BibTeX=\bfBibTeX picks a
        // font variant, meant for a group)
        if self.macro_processor.is_builtin(&target)
            && matches!(&source, Token::Command(name) if self.state.macros.contains_key(name))
        {
            return Ok(());
        }
        self.state.aliases.remove(&target);
        match source {
            Token::Command(name) if name == target => {}
            Token::Command(name) => {
                if let Some(def) = self.state.macros.get(&name).cloned() {
                    self.state.macros.insert(target, def);
                } else if matches!(name.as_str(), "relax" | "empty" | "@empty") {
                    self.state.macros.insert(target, Default::default());
                } else if let Some(def) = self.macro_processor.get_definition(&self.state, &name) {
                    let def = def.clone();
                    self.state.macros.insert(target, def);
                } else {
                    self.state.macros.remove(&target);
                    let resolved = self.state.aliases.get(&name).cloned().unwrap_or(name);
                    if resolved != target {
                        self.state.aliases.insert(target, resolved);
                    }
                }
            }
            Token::Item => {
                self.state.macros.remove(&target);
                self.state.aliases.insert(target, "item".to_string());
            }
            Token::Section(level, _, _) => {
                // Only reachable for malformed input; nothing sensible to alias
                let _ = level;
            }
            _ => {
                // \let\bgroup={ and similar character aliases: ignore
            }
        }
        Ok(())
    }

    /// Whether `name` (or `name.tex`) exists next to the file being read or
    /// the main document.
    pub(super) fn local_file_exists(&self, name: &str) -> bool {
        if name.is_empty() || name.contains('\0') {
            return false;
        }
        let mut dirs: Vec<std::path::PathBuf> = Vec::new();
        if let Some((dir, _)) = self.input_stack.last() {
            dirs.push(dir.clone());
        }
        if let Some(base) = &self.base_path {
            dirs.push(base.clone());
        }
        if dirs.is_empty() {
            dirs.push(std::path::PathBuf::new());
        }
        dirs.iter().any(|dir| {
            dir.join(name).is_file() || dir.join(format!("{}.tex", name)).is_file()
        })
    }

    /// `\docsvlist{a,b}` -> `\do{a}\do{b}`; `\forcsvlist{\h}{a,b}`;
    /// etoolbox lists built with `\listadd` and run with `\dolistloop` /
    /// `\forlistloop`; the kernel's `\@for\x:=a,b\do{body}`.
    fn parse_list_loop(&mut self, cmd: &str) -> Result<()> {
        const SEP: char = '\u{1f}';
        let handler = |parser: &mut Self| -> String {
            parser.skip_whitespace_only();
            let tokens = parser.take_branch_tokens();
            parser.raw_token_text(&tokens).trim().to_string()
        };
        let (handler, items): (String, Vec<String>) = match cmd {
            "listadd" | "listgadd" | "listeadd" | "listxadd" | "listcsadd" | "listcsgadd" => {
                self.skip_whitespace_only();
                let name = if cmd.starts_with("listcs") {
                    self.parse_braced_text()?.trim().to_string()
                } else {
                    let tokens = self.take_branch_tokens();
                    self.raw_token_text(&tokens).trim().trim_start_matches('\\').to_string()
                };
                self.skip_whitespace_only();
                let tokens = self.take_branch_tokens();
                let mut item = self.raw_token_text(&tokens);
                if matches!(cmd, "listeadd" | "listxadd") {
                    item = self.expand_parameterless_macros(&item);
                }
                if !name.is_empty() && !item.trim().is_empty() {
                    let mut body = self.state.macros.get(&name).map(|d| d.body.clone()).unwrap_or_default();
                    if !body.is_empty() {
                        body.push(SEP);
                    }
                    body.push_str(&item);
                    if body.len() <= 1 << 20 {
                        self.state.macros.insert(name, crate::state::MacroDef { body, literal: false, ..Default::default() });
                    }
                }
                return Ok(());
            }
            "docsvlist" | "forcsvlist" => {
                let handler = if cmd == "forcsvlist" { handler(self) } else { String::from("\\do") };
                self.skip_whitespace_only();
                let tokens = self.take_branch_tokens();
                let text = self.raw_token_text(&tokens);
                let text = if text.trim_start().starts_with('\\') && !text.contains(',') {
                    self.expand_parameterless_macros(&text)
                } else {
                    text
                };
                (handler, split_top_level(&text, ','))
            }
            "dolistloop" | "forlistloop" | "dolistcsloop" | "forlistcsloop" => {
                let handler = if cmd.starts_with("for") { handler(self) } else { String::from("\\do") };
                self.skip_whitespace_only();
                let name = if cmd.contains("cs") {
                    self.parse_braced_text()?.trim().to_string()
                } else {
                    let tokens = self.take_branch_tokens();
                    self.raw_token_text(&tokens).trim().trim_start_matches('\\').to_string()
                };
                let body = self.state.macros.get(&name).map(|d| d.body.clone()).unwrap_or_default();
                (handler, body.split(SEP).map(str::to_string).collect())
            }
            _ => {
                // \@for\x:=list\do{body}
                self.skip_whitespace_only();
                let Token::Command(var) = self.current_token().clone() else {
                    return Ok(());
                };
                self.advance();
                let mut list = Vec::new();
                let mut guard = 0;
                while !self.is_at_end() && guard < 10_000 {
                    guard += 1;
                    if matches!(self.current_token(), Token::Command(c) if c == "do") {
                        self.advance();
                        break;
                    }
                    list.push(self.current_token().clone());
                    self.advance();
                }
                let text = self.raw_token_text(&list);
                let text = text.trim_start().strip_prefix(":=").unwrap_or(&text).to_string();
                let text = if text.trim_start().starts_with('\\') {
                    self.expand_parameterless_macros(&text)
                } else {
                    text
                };
                self.skip_whitespace_only();
                let body_tokens = self.take_branch_tokens();
                let body = self.raw_token_text(&body_tokens);
                let mut source = String::new();
                for item in split_top_level(&text, ',') {
                    source.push_str(&format!("\\def\\{}{{{}}}{}", var, item, body));
                }
                if self.consume_expansion_bytes("@for", source.len()) && self.consume_expansion_budget("@for") {
                    self.push_front(lex_fragment(&source));
                }
                return Ok(());
            }
        };
        let mut source = String::new();
        for item in items.iter().map(|i| i.trim()).filter(|i| !i.is_empty()) {
            source.push_str(&format!("{}{{{}}}", handler, item));
        }
        if !source.is_empty() && self.consume_expansion_bytes(cmd, source.len()) && self.consume_expansion_budget(cmd) {
            self.push_front(lex_fragment(&source));
        }
        Ok(())
    }

    /// One round of a `\loop`: its body, then (on the true branch of the
    /// body's final test) the next round, then the test's `\fi`.
    fn push_loop_iteration(&mut self, id: usize) {
        let Some((body, rounds)) = self.loop_bodies.get_mut(id) else {
            return;
        };
        *rounds += 1;
        if *rounds > 10_000 || body.is_empty() {
            return;
        }
        let mut tokens = body.clone();
        if !self.consume_expansion_budget("loop") {
            return;
        }
        tokens.push(Token::Command(format!("texed@loop:{}", id)));
        tokens.push(Token::Command(String::from("fi")));
        self.push_front(tokens);
    }

    /// `\expandafter A B`: expand B once, then read A again.
    pub(super) fn expand_after(&mut self, depth: usize) -> Result<()> {
        if self.is_at_end() || depth > 64 {
            return Ok(());
        }
        // spaces after a control word are not tokens in TeX
        self.skip_whitespace_only();
        let first = self.current_token().clone();
        self.advance();
        if matches!(first, Token::Command(_)) {
            self.skip_whitespace_only();
        }
        self.expand_once(depth)?;
        self.push_front(vec![first]);
        Ok(())
    }

    /// One-level expansion of the current token (see `expand_once`).
    pub(super) fn expand_once_public(&mut self) -> Result<()> {
        if matches!(self.current_token(), Token::Command(c) if self.state.macros.contains_key(c)) {
            self.expand_once(0)
        } else {
            Ok(())
        }
    }

    /// Replace the current token by its one-level expansion, where the
    /// engine knows it: `\csname...\endcsname`, `\expandafter`, a user
    /// macro, a `\let` alias. Anything else is left as is.
    fn expand_once(&mut self, depth: usize) -> Result<()> {
        let Token::Command(name) = self.current_token().clone() else {
            return Ok(());
        };
        match name.as_str() {
            "csname" => {
                self.advance();
                let name = self.collect_csname();
                self.push_front(vec![Token::Command(if name.is_empty() { String::from("relax") } else { name })]);
            }
            "expandafter" => {
                self.advance();
                self.expand_after(depth + 1)?;
            }
            _ if self.state.aliases.contains_key(&name) => {
                let target = self.state.aliases[&name].clone();
                if self.consume_expansion_budget(&name) {
                    self.advance();
                    self.push_front(vec![Token::Command(target)]);
                }
            }
            _ if self.macro_processor.is_defined(&self.state, &name) && !self.state.macros.get(&name).is_some_and(|d| d.literal) => {
                self.advance();
                if let Some(Inline::Text(text)) = self.parse_macro_expansion(&name)? {
                    self.push_front(vec![Token::Text(text)]);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Collect a control sequence name up to `\endcsname`.
    fn collect_csname(&mut self) -> String {
        let mut name = String::new();
        for _ in 0..256 {
            if self.is_at_end() {
                break;
            }
            let token = self.current_token().clone();
            self.advance();
            match token {
                Token::Command(c) if c == "endcsname" => break,
                // \csname a\expandafter\endcsname\csname b\endcsname
                Token::Command(c) if c == "expandafter" => {
                    let _ = self.expand_after(0);
                }
                Token::Command(c) => {
                    // \csname\string\foo\endcsname and similar: expand to name
                    if let Some(def) = self.state.macros.get(&c) {
                        name.push_str(def.body.trim());
                    } else {
                        name.push_str(&c);
                    }
                }
                Token::Whitespace(_) | Token::Newline => {}
                other => name.push_str(&self.token_to_string(&other)),
            }
        }
        name
    }

    /// `\the\value{x}`, `\arabic{x}`, `\roman{x}`, `\thesection`, ...
    fn parse_counter_format(&mut self, cmd: &str) -> Result<Option<Inline>> {
        self.skip_whitespace_and_comments();
        match cmd {
            "the" | "number" | "romannumeral" => {
                let token = self.current_token().clone();
                match token {
                    Token::Command(name) => {
                        self.advance();
                        // Register accessors take an operand (\the\dp88,
                        // \the\count255): read it so it does not leak
                        if matches!(name.as_str(), "wd" | "ht" | "dp" | "dimen" | "skip" | "toks" | "count" | "muskip") {
                            self.skip_whitespace_only();
                            if matches!(self.current_token(), Token::Command(_)) {
                                self.advance();
                            } else {
                                let _ = self.read_tex_number();
                            }
                            return Ok(None);
                        }
                        let value = self.command_numeric_value(&name);
                        if let Some(counter) = name.strip_prefix("the") {
                            if let Some(text) = self.format_named_counter(counter, "arabic") {
                                return Ok(Some(Inline::Text(text)));
                            }
                        }
                        Ok(value.map(|v| {
                            Inline::Text(if cmd == "romannumeral" {
                                roman_number(v.max(0) as usize, false)
                            } else {
                                v.to_string()
                            })
                        }))
                    }
                    Token::Text(_) if cmd != "the" => {
                        let n = self.read_tex_number();
                        Ok(n.map(|v| {
                            Inline::Text(if cmd == "romannumeral" {
                                roman_number(v.max(0) as usize, false)
                            } else {
                                v.to_string()
                            })
                        }))
                    }
                    _ => Ok(None),
                }
            }
            _ => {
                let counter = self.parse_braced_text()?.trim().to_string();
                if cmd == "value" {
                    return Ok(Some(Inline::Text(self.counter_value(&counter).to_string())));
                }
                Ok(self
                    .format_named_counter(&counter, cmd)
                    .map(Inline::Text))
            }
        }
    }

    /// Format a counter in a numbering style; `None` if the counter is unknown.
    fn format_named_counter(&self, counter: &str, style: &str) -> Option<String> {
        let known = matches!(
            counter,
            "chapter" | "section" | "subsection" | "subsubsection" | "paragraph"
                | "subparagraph" | "equation" | "figure" | "table" | "footnote" | "page"
        ) || self.state.user_counters.contains_key(counter);
        if !known {
            return None;
        }
        if style == "arabic" {
            if let Some(level) = crate::state::level_of_sectioning(counter) {
                return Some(self.state.section_number_up_to(level));
            }
        }
        let n = if counter == "page" { 1 } else { self.counter_value(counter) };
        Some(match style {
            "roman" => roman_number(n, false),
            "Roman" => roman_number(n, true),
            "alph" => alpha_number(n, false),
            "Alph" => alpha_number(n, true),
            "fnsymbol" => ["*", "†", "‡", "§", "¶", "‖", "**", "††", "‡‡"]
                .get(n.saturating_sub(1))
                .unwrap_or(&"*")
                .to_string(),
            _ => n.to_string(),
        })
    }

    /// Account one expansion of `name` against the global and per-macro
    /// budgets and the nesting limits. Returns false (and warns once) when
    /// the expansion must be refused because it is runaway recursion.
    pub(super) fn consume_expansion_budget(&mut self, name: &str) -> bool {
        // Leave expansions whose tokens have been fully consumed
        let len = self.tokens.len();
        while let Some((_, base)) = self.active_expansions.last() {
            if *base >= len {
                self.active_expansions.pop();
            } else {
                break;
            }
        }

        let nesting = self
            .active_expansions
            .iter()
            .filter(|(active, _)| active == name)
            .count();
        let count = match self.expansion_counts.get_mut(name) {
            Some(count) => {
                *count += 1;
                *count
            }
            None => {
                self.expansion_counts.insert(name.to_string(), 1);
                1
            }
        };
        let per_macro_limit = PER_MACRO_BASE_BUDGET
            .saturating_add(self.source_token_count.saturating_mul(2))
            .saturating_add(
                self.source_command_counts
                    .get(name)
                    .copied()
                    .unwrap_or(0)
                    .saturating_mul(64),
            );

        let refused = nesting >= MAX_SAME_MACRO_NESTING
            || self.active_expansions.len() >= MAX_EXPANSION_NESTING
            || count > per_macro_limit
            || self.macro_expansions >= self.macro_budget
            || self.tokens.len() > super::MAX_PENDING_TOKENS;
        if refused {
            if self.refused_macros.insert(name.to_string()) {
                self.state.warn(format!(
                    "recursive or runaway macro `\\{}` — expansion stopped",
                    name
                ));
            }
            return false;
        }
        self.macro_expansions += 1;
        self.active_expansions.push((name.to_string(), len));
        true
    }
}

/// Lex a macro body / fragment into tokens (without the trailing Eof).
///
/// Bodies are stored as text, so the `\makeatletter` state in force when a
/// macro was defined is gone; a body that refers to `\@name` control
/// sequences was necessarily defined with `@` as a letter, so it is
/// re-lexed that way.
pub(super) fn lex_fragment(fragment: &str) -> Vec<Token> {
    let at_letter = uses_at_names(fragment);
    let expl = uses_expl_names(fragment);
    let mut tokens = Lexer::new(fragment)
        .with_at_letter(at_letter)
        .with_expl_syntax(expl)
        .tokenize()
        .unwrap_or_default();
    if matches!(tokens.last(), Some(Token::Eof)) {
        tokens.pop();
    }
    drop_spaces_after_control_words(tokens)
}

/// TeX's tokenizer skips the spaces, and one line end, after a control
/// word: `\def\x{\foo bar}` is `\foo` then `bar`. Applied to macro bodies
/// and other re-read fragments (document text keeps its spaces, which is
/// kinder to `\LaTeX is` typos).
fn drop_spaces_after_control_words(tokens: Vec<Token>) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    let mut iter = tokens.into_iter().peekable();
    while let Some(token) = iter.next() {
        let word = matches!(&token, Token::Command(name) if name.ends_with(|c: char| c.is_alphabetic() || c == '@'));
        out.push(token);
        if !word {
            continue;
        }
        // the spaces (and one line end) after a control word
        let mut skipped = Vec::new();
        let mut newline_seen = false;
        while let Some(next) = iter.peek() {
            match next {
                Token::Whitespace(_) => skipped.push(iter.next().unwrap()),
                Token::Newline if !newline_seen => {
                    newline_seen = true;
                    skipped.push(iter.next().unwrap());
                }
                _ => break,
            }
        }
        // keep one space before letters: token lists are written back to
        // text, where `\foo bar` must not become `\foobar`
        if !skipped.is_empty() {
            if let Some(Token::Text(t)) = iter.peek() {
                if t.starts_with(|c: char| c.is_alphabetic() || c == '@') {
                    out.push(Token::Whitespace(String::from(" ")));
                }
            }
        }
    }
    out
}

/// Does the text use expl3 function names (`\clist_set:Nx`)? Such text
/// was written under `\ExplSyntaxOn`.
pub(super) fn uses_expl_names(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            let start = i + 1;
            let mut j = start;
            let mut underscore = false;
            while j < bytes.len() && (bytes[j].is_ascii_alphabetic() || bytes[j] == b'_') {
                underscore |= bytes[j] == b'_';
                j += 1;
            }
            // `\__private:n` or `\module_name:n`, not an escaped `\_` before
            // a word and a colon (`\_foo: bar` in text)
            let name = &bytes[start..j];
            let shape_ok = match name.first() {
                Some(b'_') => name.starts_with(b"__") && name.len() > 2 && name[2].is_ascii_alphabetic(),
                Some(c) => c.is_ascii_alphabetic(),
                None => false,
            };
            if underscore && shape_ok && j < bytes.len() && bytes[j] == b':' && j > start + 1 {
                return true;
            }
            // a kernel function without an underscore (`\use:c`, `\use:n`):
            // the colon is followed by an argument signature
            if shape_ok && j < bytes.len() && bytes[j] == b':' && j > start + 1 {
                let sig: Vec<u8> = bytes[j + 1..].iter().take_while(|b| b.is_ascii_alphabetic()).copied().collect();
                if !sig.is_empty() && sig.len() <= 6 && sig.iter().all(|b| b"NncVvoxefTFpwD".contains(b)) {
                    return true;
                }
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    false
}

/// Does the text refer to `@`-named control sequences (`\@foo`,
/// `\ol@sectioncs`)? Such text was written with `@` as a letter.
pub(super) fn uses_at_names(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            let start = i + 1;
            let mut j = start;
            let mut has_at = false;
            while j < bytes.len() && (bytes[j].is_ascii_alphabetic() || bytes[j] == b'@') {
                has_at |= bytes[j] == b'@';
                j += 1;
            }
            // `\@` alone (spacing) does not count; `\@foo` and `\a@b` do
            if has_at && j - start >= 2 {
                return true;
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    false
}

/// Parse a TeX number or dimension (`12`, `-3`, `2.5pt`, `1em`) to a value
/// in points (units other than pt are converted approximately).
fn parse_tex_dimen(text: &str) -> Option<f64> {
    let text = text.trim();
    let numeric: String = text
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | ','))
        .collect();
    let value: f64 = numeric.replace(',', ".").parse().ok()?;
    let unit = text[numeric.len()..].trim();
    let factor = match unit {
        "" | "pt" | "sp" => 1.0,
        "in" => 72.27,
        "cm" => 28.45,
        "mm" => 2.845,
        "bp" => 1.00375,
        "pc" => 12.0,
        "em" => 10.0,
        "ex" => 4.3,
        _ => 1.0,
    };
    Some(value * factor)
}

fn num(text: &str) -> Option<i64> {
    text.trim().parse().ok()
}

/// `\name{arg}` -> Some("arg") if `text` starts with `prefix{`.
fn braced_arg<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = text.strip_prefix(prefix)?.trim_start();
    let (arg, _) = split_braced(rest);
    rest.starts_with('{').then_some(arg)
}

/// Split a leading `{...}` group: returns (content, remainder).
fn split_braced(text: &str) -> (&str, &str) {
    let text = text.trim_start();
    if !text.starts_with('{') {
        return ("", text);
    }
    let mut depth = 0;
    for (i, ch) in text.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return (&text[1..i], &text[i + 1..]);
                }
            }
            _ => {}
        }
    }
    (&text[1..], "")
}

/// Control helpers evaluated when expanding macros inside math.
fn is_math_control_helper(name: &str) -> bool {
    matches!(
        name,
        "@ifnextchar" | "kernel@ifnextchar" | "new@ifnextchar" | "@ifstar" | "@ifundefined"
            | "@ifpackageloaded" | "@ifclassloaded" | "@firstoftwo" | "@secondoftwo"
            | "@firstofone" | "@gobble" | "@gobbletwo" | "@gobblefour" | "@iden" | "csname"
            | "else" | "fi" | "or" | "expandafter" | "noexpand" | "protect" | "relax"
            | "xspace" | "ensuremath" | "@empty" | "mathchoice" | "ifthenelse" | "iftoggle"
            | "ifbool" | "ifdef" | "ifundef" | "unskip" | "ignorespaces" | "nobreak"
            | "leavevmode" | "@nil" | "begingroup" | "endgroup" | "IfNoValueTF" | "IfNoValueT"
            | "IfNoValueF" | "IfValueTF" | "IfValueT" | "IfValueF" | "IfBooleanTF"
            | "IfBooleanT" | "IfBooleanF" | "BooleanTrue" | "BooleanFalse" | "csuse"
            | "@ifpackagelater" | "@ifclasslater" | "ifdefempty" | "ifstrequal" | "ifstrempty"
            | "ifblank" | "notbool" | "nottoggle" | "IfFileExists"
    )
}

impl Parser {
    /// Expand document macros inside a math formula with the full TeX
    /// token engine (delimited parameters, `\@ifnextchar`, conditionals,
    /// `\let` aliases), serializing the result back to math source.
    /// Commands that are not document macros are left untouched for the
    /// math renderer.
    pub(super) fn expand_math_with_engine(&mut self, math: &str, display: bool) -> String {
        let needs_engine = math.contains('\\') && {
            let bytes = math.as_bytes();
            let mut i = 0;
            let mut found = false;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    let start = i + 1;
                    let mut end = start;
                    while end < bytes.len() && (bytes[end].is_ascii_alphabetic() || bytes[end] == b'@') {
                        end += 1;
                    }
                    if end > start {
                        let name = &math[start..end];
                        if self.state.macros.contains_key(name)
                            || self.state.aliases.contains_key(name)
                            || self.is_tex_conditional(name)
                            || is_math_control_helper(name)
                        {
                            found = true;
                            break;
                        }
                    }
                    i = end.max(i + 1);
                } else {
                    i += 1;
                }
            }
            found
        };
        if !needs_engine {
            return math.to_string();
        }

        let at_letter = uses_at_names(math);
        let mut tokens = Lexer::new(math)
            .with_math_mode(true)
            .with_at_letter(at_letter)
            .tokenize()
            .unwrap_or_default();
        if matches!(tokens.last(), Some(Token::Eof)) {
            tokens.pop();
        }

        let saved_tokens = std::mem::replace(&mut self.tokens, super::reverse_tokens(tokens));
        let saved_expansions = std::mem::take(&mut self.active_expansions);
        let saved_cond = std::mem::take(&mut self.cond_depth);
        let saved_in_math = std::mem::replace(&mut self.in_math_expansion, true);

        let mut out = String::with_capacity(math.len());
        let mut guard = 0usize;
        // Limits proportional to the formula: notation macros expand a
        // formula a few times over; anything far beyond is runaway
        let step_limit = 20_000 + 400 * math.len();
        let output_limit = 16_384 + 64 * math.len();
        let budget_snapshot = (self.macro_expansions, self.expanded_bytes, self.expansion_counts.clone());
        let mut aborted = false;
        while !self.is_at_end() {
            guard += 1;
            if guard > step_limit || out.len() > output_limit || self.tokens.len() > output_limit {
                aborted = true;
                break;
            }
            let token = self.current_token().clone();
            match &token {
                Token::Command(name) if self.state.macros.get(name).is_some_and(|d| d.literal) => {
                    let body = self.state.macros[name].body.clone();
                    self.advance();
                    push_math_piece(&mut out, &body);
                }
                Token::Command(name) if self.state.macros.contains_key(name) => {
                    self.advance();
                    let expandable = self
                        .state
                        .macros
                        .get(name)
                        .is_some_and(|d| d.num_params <= 9);
                    if !expandable {
                        push_math_piece(&mut out, &format!("\\{}", name));
                        continue;
                    }
                    let args = self.parse_macro_arguments(name).unwrap_or_default();
                    let body = self
                        .state
                        .macros
                        .get(name)
                        .map(|d| crate::state::substitute_params(&d.body, &args))
                        .unwrap_or_default();
                    if self.consume_expansion_bytes(name, body.len()) && self.consume_expansion_budget(name) {
                        let at = uses_at_names(&body);
                        let mut expansion = Lexer::new(&body)
                            .with_math_mode(true)
                            .with_at_letter(at)
                            .tokenize()
                            .unwrap_or_default();
                        if matches!(expansion.last(), Some(Token::Eof)) {
                            expansion.pop();
                        }
                        // a macro body's layout (indentation, line breaks) is
                        // not the formula's: one space at most
                        let mut compact: Vec<Token> = Vec::with_capacity(expansion.len());
                        for token in expansion {
                            if matches!(token, Token::Whitespace(_) | Token::Newline) {
                                if !matches!(compact.last(), Some(Token::Whitespace(_)) | None) {
                                    compact.push(Token::Whitespace(String::from(" ")));
                                }
                            } else {
                                compact.push(token);
                            }
                        }
                        self.push_front(compact);
                    } else {
                        push_math_piece(&mut out, &format!("\\{}", name));
                    }
                }
                Token::Command(name) if self.state.aliases.contains_key(name) => {
                    self.advance();
                    let target = self.state.aliases[name].clone();
                    push_math_piece(&mut out, &format!("\\{}", target));
                }
                // TeX box/register programming: interpreted, not emitted
                Token::Command(name) if name == "setbox" => {
                    self.advance();
                    self.consume_setbox();
                }
                Token::Command(name)
                    if self.tex_registers.contains(name)
                        || crate::parser::knowledge::tex_assignment(name).is_some_and(|(_, v)| v)
                            && !matches!(name.as_str(), "kern" | "mkern" | "mskip" | "hskip" | "raise" | "lower" | "moveleft" | "moveright") =>
                {
                    let name = name.clone();
                    self.advance();
                    let operands = crate::parser::knowledge::tex_assignment(&name).map(|(o, _)| o).unwrap_or(0);
                    self.consume_tex_assignment(operands, true);
                }
                Token::Command(name) if matches!(name.as_str(), "ht" | "wd" | "dp") => {
                    self.advance();
                    // the box register
                    if matches!(self.current_token(), Token::Command(_) | Token::Text(_)) {
                        self.advance();
                    }
                }
                Token::Command(name) if matches!(name.as_str(), "raise" | "lower" | "moveleft" | "moveright") => {
                    // \raise<dimen>\hbox{...}: keep the box content only
                    self.advance();
                    let mut k = 0;
                    while k < 12 && !matches!(self.current_token(), Token::Command(c) if matches!(c.as_str(), "hbox" | "vbox" | "mbox" | "box")) {
                        if self.is_at_end() {
                            break;
                        }
                        self.advance();
                        k += 1;
                    }
                }
                // definitions and state changes inside math (a macro used in
                // a formula that defines a helper first): run, emit nothing
                Token::Command(name)
                    if matches!(
                        name.as_str(),
                        "def" | "gdef" | "edef" | "xdef" | "let" | "global" | "newcommand" | "renewcommand"
                            | "providecommand" | "DeclareRobustCommand" | "newtoggle" | "providetoggle"
                            | "toggletrue" | "togglefalse" | "settoggle" | "docsvlist" | "forcsvlist"
                            | "listadd" | "dolistloop" | "forlistloop" | "@for" | "setcounter"
                            | "addtocounter" | "stepcounter" | "refstepcounter" | "newif" | "setboolean"
                            | "newboolean"
                    ) && !self.state.macros.contains_key(name) =>
                {
                    let name = name.clone();
                    let _ = self.parse_inline_command(name);
                }
                Token::Command(name) if name == "mathchoice" => {
                    self.advance();
                    let mut styles = Vec::new();
                    for _ in 0..4 {
                        styles.push(self.take_branch_tokens());
                    }
                    let chosen = std::mem::take(&mut styles[if display { 0 } else { 1 }]);
                    self.push_front(chosen);
                }
                Token::Command(name)
                    if matches!(name.as_str(), "xspace" | "protect" | "relax" | "ensuremath"
                        | "expandafter" | "noexpand" | "unskip" | "ignorespaces" | "nobreak"
                        | "leavevmode" | "@empty" | "@nil" | "begingroup" | "endgroup") =>
                {
                    self.advance();
                }
                Token::Command(name)
                    if is_math_control_helper(name) || self.is_tex_conditional(name) =>
                {
                    let name = name.clone();
                    self.advance();
                    let _ = self.parse_tex_command(&name);
                }
                other => {
                    let piece = self.token_to_string(other);
                    self.advance();
                    push_math_piece(&mut out, &piece);
                }
            }
        }
        if aborted {
            // Runaway expansion: keep the formula as written, and give the
            // budget back so the rest of the document is unaffected
            self.tokens.clear();
            self.macro_expansions = budget_snapshot.0;
            self.expanded_bytes = budget_snapshot.1;
            self.expansion_counts = budget_snapshot.2;
            out = math.to_string();
        }

        self.tokens = saved_tokens;
        self.active_expansions = saved_expansions;
        self.cond_depth = saved_cond;
        self.in_math_expansion = saved_in_math;
        if out != math {
            // spaces are insignificant in math: tidy what expansion left
            out = out
                .lines()
                .map(|line| {
                    let mut tidy = String::with_capacity(line.len());
                    for c in line.trim_end().chars() {
                        if c == ' ' && tidy.ends_with(' ') && !tidy.ends_with("\\ ") {
                            continue;
                        }
                        tidy.push(c);
                    }
                    tidy
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
        out
    }
}

/// Append serialized math, separating a control word from a following
/// letter so `\alpha` + `b` never fuses into `\alphab`.
fn push_math_piece(out: &mut String, piece: &str) {
    if piece.chars().next().is_some_and(|c| c.is_alphabetic()) {
        let tail: Vec<char> = out.chars().rev().take_while(|c| c.is_alphabetic() || *c == '@').collect();
        if !tail.is_empty() && out[..out.len() - tail.iter().map(|c| c.len_utf8()).sum::<usize>()].ends_with('\\') {
            out.push(' ');
        }
    }
    out.push_str(piece);
}

/// Split at `sep` outside braces.
fn split_top_level(text: &str, sep: char) -> Vec<String> {
    let mut items = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for c in text.chars() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            c if c == sep && depth == 0 => {
                items.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    items.push(current);
    items
}
