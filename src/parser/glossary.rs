//! Acronyms and glossary entries (glossaries, acronym, acro packages).
//!
//! Definitions are recorded; uses render as LaTeX would: the first use of
//! an acronym gives "long form (short)", later uses the short form.

use super::{Inline, Parser};
use crate::error::Result;
use crate::lexer::Token;

#[derive(Debug, Clone, Default)]
pub(super) struct GlossaryEntry {
    pub short: String,
    pub long: Option<String>,
    pub plural: Option<String>,
    pub long_plural: Option<String>,
    pub description: Option<String>,
    pub symbol: Option<String>,
    pub used: bool,
}

/// Parse `key=value, key={value, with commas}` option lists.
fn parse_keyvals(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut current = String::new();
    let mut items = Vec::new();
    for ch in text.chars() {
        match ch {
            '{' => {
                depth += 1;
                current.push(ch);
            }
            '}' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => items.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    items.push(current);
    for item in items {
        if let Some((key, value)) = item.split_once('=') {
            let value = value.trim();
            let value = value
                .strip_prefix('{')
                .and_then(|v| v.strip_suffix('}'))
                .unwrap_or(value);
            out.push((key.trim().to_string(), value.trim().to_string()));
        }
    }
    out
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

impl Parser {
    /// Handle glossary/acronym definitions and uses.
    pub(super) fn parse_glossary_command(&mut self, cmd: &str) -> Result<Option<Option<Inline>>> {
        match cmd {
            // \newacronym[opts]{key}{short}{long}
            "newacronym" | "newabbreviation" => {
                self.skip_whitespace_and_comments();
                let options = self.parse_bracketed_text()?;
                self.skip_whitespace_and_comments();
                let key = self.parse_braced_text()?;
                self.skip_whitespace_and_comments();
                let short = self.parse_braced_text()?;
                self.skip_whitespace_and_comments();
                let long = self.parse_braced_text()?;
                let mut entry = GlossaryEntry {
                    short: short.trim().to_string(),
                    long: Some(long.trim().to_string()),
                    ..Default::default()
                };
                for (k, v) in parse_keyvals(&options) {
                    match k.as_str() {
                        "shortplural" => entry.plural = Some(v),
                        "longplural" => entry.long_plural = Some(v),
                        "description" => entry.description = Some(v),
                        _ => {}
                    }
                }
                self.glossary.insert(key.trim().to_string(), entry);
                Ok(Some(None))
            }
            // \acro{key}[short]{long} / \acrodef (acronym package)
            "acro" | "acrodef" | "newacro" => {
                self.skip_whitespace_and_comments();
                let key = self.parse_braced_text()?;
                self.skip_whitespace_and_comments();
                let short = if matches!(self.current_token(), Token::LeftBracket) {
                    self.parse_bracketed_text()?
                } else {
                    key.clone()
                };
                self.skip_whitespace_and_comments();
                let long = self.parse_braced_text()?;
                self.glossary.insert(
                    key.trim().to_string(),
                    GlossaryEntry {
                        short: short.trim().to_string(),
                        long: Some(long.trim().to_string()),
                        ..Default::default()
                    },
                );
                Ok(Some(None))
            }
            // \DeclareAcronym{key}{short=..., long=...} (acro package)
            "DeclareAcronym" => {
                self.skip_whitespace_and_comments();
                let key = self.parse_braced_text()?;
                self.skip_whitespace_and_comments();
                let options = self.parse_braced_text()?;
                let mut entry = GlossaryEntry::default();
                for (k, v) in parse_keyvals(&options) {
                    match k.as_str() {
                        "short" => entry.short = v,
                        "long" => entry.long = Some(v),
                        "short-plural" | "short-plural-form" => entry.plural = Some(v),
                        "long-plural" | "long-plural-form" => entry.long_plural = Some(v),
                        _ => {}
                    }
                }
                if entry.short.is_empty() {
                    entry.short = key.trim().to_string();
                }
                self.glossary.insert(key.trim().to_string(), entry);
                Ok(Some(None))
            }
            // \newglossaryentry{key}{name=..., description=..., plural=...}
            "newglossaryentry" | "longnewglossaryentry" | "provideglossaryentry" => {
                self.skip_whitespace_and_comments();
                let key = self.parse_braced_text()?;
                self.skip_whitespace_and_comments();
                let options = self.parse_braced_text()?;
                if cmd == "longnewglossaryentry" {
                    self.skip_whitespace_and_comments();
                    let _ = self.parse_braced_text()?;
                }
                let mut entry = GlossaryEntry::default();
                for (k, v) in parse_keyvals(&options) {
                    match k.as_str() {
                        "name" => entry.short = v,
                        "text" if entry.short.is_empty() => entry.short = v,
                        "plural" => entry.plural = Some(v),
                        "description" => entry.description = Some(v),
                        "symbol" => entry.symbol = Some(v),
                        "first" => entry.long = Some(v),
                        _ => {}
                    }
                }
                if entry.short.is_empty() {
                    entry.short = key.trim().to_string();
                }
                // A plain glossary entry's first use is just its name
                self.glossary.insert(key.trim().to_string(), entry);
                Ok(Some(None))
            }
            "glsresetall" | "acresetall" | "glsunsetall" | "acuseall" => {
                let used = cmd == "glsunsetall" || cmd == "acuseall";
                for entry in self.glossary.values_mut() {
                    entry.used = used;
                }
                Ok(Some(None))
            }
            "glsreset" | "acreset" | "glsunset" | "acused" => {
                self.skip_whitespace_and_comments();
                let key = self.parse_braced_text()?;
                if let Some(entry) = self.glossary.get_mut(key.trim()) {
                    entry.used = cmd == "glsunset" || cmd == "acused";
                }
                Ok(Some(None))
            }
            _ => {
                let Some(form) = use_form(cmd) else {
                    return Ok(None);
                };
                // Uses: \gls[opts]{key}[insert]
                self.consume_text_star();
                self.skip_whitespace_only();
                if matches!(self.current_token(), Token::LeftBracket) {
                    let _ = self.parse_bracketed_text()?;
                }
                self.skip_whitespace_only();
                if !matches!(self.current_token(), Token::LeftBrace) {
                    return Ok(None);
                }
                let key = self.parse_braced_text()?.trim().to_string();
                let insert = if matches!(self.current_token(), Token::LeftBracket) {
                    self.parse_bracketed_text()?
                } else {
                    String::new()
                };
                let text = match self.glossary.get_mut(&key) {
                    Some(entry) => {
                        let text = render_use(entry, &form);
                        if form.marks_used {
                            entry.used = true;
                        }
                        text
                    }
                    None => {
                        self.state
                            .warn(format!("undefined glossary entry or acronym `{}`", key));
                        key.clone()
                    }
                };
                let text = if form.capitalize { capitalize(&text) } else { text };
                let text = if form.upper { text.to_uppercase() } else { text };
                // The text is LaTeX (entries may hold markup): parse it
                let (inlines, _) = self.parse_inline_fragment(&format!("{}{}", text, insert))?;
                Ok(Some(Some(super::span_or_single(inlines))))
            }
        }
    }
}

/// Which representation of an entry a command asks for.
struct UseForm {
    kind: UseKind,
    plural: bool,
    capitalize: bool,
    upper: bool,
    marks_used: bool,
}

#[derive(PartialEq)]
enum UseKind {
    /// First use gives the full form, later uses the short form
    Auto,
    Short,
    Long,
    Full,
    Description,
    Symbol,
}

fn use_form(cmd: &str) -> Option<UseForm> {
    let (kind, plural, capitalize, upper, marks_used) = match cmd {
        "gls" | "ac" => (UseKind::Auto, false, false, false, true),
        "Gls" | "Ac" => (UseKind::Auto, false, true, false, true),
        "GLS" => (UseKind::Auto, false, false, true, true),
        "glspl" | "acp" => (UseKind::Auto, true, false, false, true),
        "Glspl" | "Acp" => (UseKind::Auto, true, true, false, true),
        "GLSpl" => (UseKind::Auto, true, false, true, true),
        "acrshort" | "acs" | "glsentryshort" | "acsu" | "glsshort" | "glsxtrshort"
        | "glsentrytext" | "glstext" | "glsname" | "glsentryname" => {
            (UseKind::Short, false, false, false, false)
        }
        "Acrshort" | "Glstext" | "Glsentrytext" | "Glsname" | "Glsentryname" | "Acs" => {
            (UseKind::Short, false, true, false, false)
        }
        "acrshortpl" | "acsp" | "glsplural" | "glsentryplural" => (UseKind::Short, true, false, false, false),
        "acrlong" | "acl" | "glsentrylong" | "glslong" | "glsxtrlong" | "aclu" => {
            (UseKind::Long, false, false, false, false)
        }
        "Acrlong" | "Acl" | "Glsentrylong" => (UseKind::Long, false, true, false, false),
        "acrlongpl" | "aclp" => (UseKind::Long, true, false, false, false),
        "Acrlongpl" | "Aclp" => (UseKind::Long, true, true, false, false),
        "acrfull" | "acf" | "glsentryfull" | "glsxtrfull" | "acfi" => (UseKind::Full, false, false, false, true),
        "Acrfull" | "Acf" => (UseKind::Full, false, true, false, true),
        "acrfullpl" | "acfp" => (UseKind::Full, true, false, false, true),
        "Acrfullpl" | "Acfp" => (UseKind::Full, true, true, false, true),
        "glsdesc" | "glsentrydesc" => (UseKind::Description, false, false, false, false),
        "Glsdesc" | "Glsentrydesc" => (UseKind::Description, false, true, false, false),
        "glssymbol" | "glsentrysymbol" => (UseKind::Symbol, false, false, false, false),
        _ => return None,
    };
    Some(UseForm {
        kind,
        plural,
        capitalize,
        upper,
        marks_used,
    })
}

fn render_use(entry: &GlossaryEntry, form: &UseForm) -> String {
    let short = if form.plural {
        entry.plural.clone().unwrap_or_else(|| format!("{}s", entry.short))
    } else {
        entry.short.clone()
    };
    let long = entry.long.as_ref().map(|long| {
        if form.plural {
            entry.long_plural.clone().unwrap_or_else(|| format!("{}s", long))
        } else {
            long.clone()
        }
    });
    match form.kind {
        UseKind::Short => short,
        UseKind::Long => long.unwrap_or(short),
        UseKind::Full => match long {
            Some(long) => format!("{} ({})", long, short),
            None => short,
        },
        UseKind::Auto => match long {
            Some(long) if !entry.used => format!("{} ({})", long, short),
            _ => short,
        },
        UseKind::Description => entry.description.clone().unwrap_or_default(),
        UseKind::Symbol => entry.symbol.clone().unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    fn convert(latex: &str) -> String {
        crate::convert_with(
            latex,
            &crate::Options {
                no_frontmatter: true,
                ..Default::default()
            },
        )
        .unwrap()
        .markdown
    }

    #[test]
    fn glossaries_first_use_then_short() {
        let md = convert("\\newacronym{cpu}{CPU}{Central Processing Unit} \\gls{cpu} then \\gls{cpu}, \\glspl{cpu}, \\acrlong{cpu}, \\Gls{cpu}");
        assert!(
            md.contains("Central Processing Unit (CPU) then CPU, CPUs, Central Processing Unit, CPU"),
            "got: {}",
            md
        );
    }

    #[test]
    fn acronym_package() {
        let md = convert("\\begin{acronym}\\acro{GPU}{Graphics Processing Unit}\\end{acronym} \\ac{GPU} and \\ac{GPU}; \\acf{GPU}");
        assert!(md.contains("Graphics Processing Unit (GPU) and GPU; Graphics Processing Unit (GPU)"), "got: {}", md);
    }

    #[test]
    fn glossary_entries() {
        let md = convert("\\newglossaryentry{lat}{name={lattice}, description={a partially ordered set}} A \\gls{lat}: \\glsdesc{lat}.");
        assert!(md.contains("A lattice: a partially ordered set."), "got: {}", md);
    }

    #[test]
    fn acro_package() {
        let md = convert("\\DeclareAcronym{ny}{short = NY, long = New York} \\ac{ny}, \\ac{ny}");
        assert!(md.contains("New York (NY), NY"), "got: {}", md);
    }
}
