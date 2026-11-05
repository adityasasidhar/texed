//! Expansion of document macros inside math, so formulas render in
//! KaTeX/MathJax (which know nothing of a document's `\newcommand`s).
//!
//! Expansion is recursive with cycle detection: a macro is never expanded
//! inside its own expansion, depth and output size are bounded, and macros
//! whose bodies are TeX programming (`\@ifnextchar`, `\futurelet`, ...) are
//! left alone because their expansion is meaningless to a math renderer.

use crate::state::{substitute_params, MacroDef};
use std::collections::HashMap;

const MAX_DEPTH: usize = 24;
const MAX_OUTPUT: usize = 100_000;

/// Fragments marking a macro body as TeX programming rather than notation.
const PROGRAMMING_MARKERS: &[&str] = &[
    "\\@", "\\if", "\\futurelet", "\\expandafter", "\\csname", "\\def", "\\gdef", "\\edef",
    "\\let", "\\afterassignment", "\\begingroup", "\\endgroup", "\\aftergroup", "\\global",
    "\\newcommand", "\\renewcommand", "\\IfNoValue", "\\IfValue", "\\IfBoolean", "\\peek_",
    "\\NewDocumentCommand", "\\setbox", "\\hbox", "\\vbox", "\\count", "\\advance",
];

pub(super) fn expand_math(content: &str, macros: &HashMap<String, MacroDef>) -> String {
    let mut stack = Vec::new();
    let expanded = if macros.is_empty() {
        content.to_string()
    } else {
        expand(content, macros, &mut stack, 0)
    };
    clean_math(&expanded)
}

fn is_expandable(def: &MacroDef) -> bool {
    def.delimiters.iter().all(|d| d.trim().is_empty())
        && def.prefix.trim().is_empty()
        && !PROGRAMMING_MARKERS.iter().any(|m| def.body.contains(m))
}

fn expand(
    text: &str,
    macros: &HashMap<String, MacroDef>,
    stack: &mut Vec<String>,
    depth: usize,
) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if out.len() > MAX_OUTPUT {
            out.extend(&chars[i..]);
            break;
        }
        if chars[i] != '\\' || i + 1 >= chars.len() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        if !chars[i + 1].is_alphabetic() {
            // control symbol: copy both characters
            out.push(chars[i]);
            out.push(chars[i + 1]);
            i += 2;
            continue;
        }
        let start = i + 1;
        let mut end = start;
        while end < chars.len() && chars[end].is_alphabetic() {
            end += 1;
        }
        let name: String = chars[start..end].iter().collect();
        let def = macros.get(&name);
        let usable = def.is_some_and(is_expandable)
            && depth < MAX_DEPTH
            && !stack.iter().any(|active| active == &name);
        let Some(def) = def.filter(|_| usable) else {
            out.extend(&chars[i..end]);
            i = end;
            continue;
        };

        let (args, next) = read_args(&chars, end, def);
        let body = substitute_params(&def.body, &args);
        stack.push(name);
        let expanded = expand(&body, macros, stack, depth + 1);
        stack.pop();
        // Keep a following letter from fusing with a control word the
        // expansion ends with (\R x -> \mathbb{R}x is fine, \foo -> \alpha
        // followed by "b" must not become \alphab)
        let ends_with_control_word = {
            let tail: Vec<char> = expanded.chars().rev().take_while(|c| c.is_alphabetic()).collect();
            !tail.is_empty() && expanded[..expanded.len() - tail.len()].ends_with('\\')
        };
        out.push_str(&expanded);
        if ends_with_control_word && next < chars.len() && chars[next].is_alphabetic() {
            out.push(' ');
        }
        i = next;
    }
    out
}

/// Read a macro's arguments starting at `j`; returns them and the index
/// after the last one.
fn read_args(chars: &[char], mut j: usize, def: &MacroDef) -> (Vec<String>, usize) {
    let mut args = Vec::new();
    let required = if let Some(default) = &def.optional_param {
        let mut k = j;
        while k < chars.len() && chars[k] == ' ' {
            k += 1;
        }
        if k < chars.len() && chars[k] == '[' {
            let mut depth = 0;
            let mut m = k + 1;
            let mut opt = String::new();
            while m < chars.len() {
                match chars[m] {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    ']' if depth == 0 => break,
                    _ => {}
                }
                opt.push(chars[m]);
                m += 1;
            }
            args.push(opt);
            j = (m + 1).min(chars.len());
        } else {
            args.push(default.clone());
        }
        def.num_params.saturating_sub(1)
    } else {
        def.num_params
    };

    for _ in 0..required {
        while j < chars.len() && chars[j].is_whitespace() {
            j += 1;
        }
        if j >= chars.len() {
            args.push(String::new());
            continue;
        }
        if chars[j] == '{' {
            let mut depth = 1;
            let mut k = j + 1;
            let mut arg = String::new();
            while k < chars.len() {
                match chars[k] {
                    '\\' if k + 1 < chars.len() => {
                        arg.push(chars[k]);
                        arg.push(chars[k + 1]);
                        k += 2;
                        continue;
                    }
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                arg.push(chars[k]);
                k += 1;
            }
            args.push(arg);
            j = (k + 1).min(chars.len());
        } else if chars[j] == '\\' && j + 1 < chars.len() {
            // a control sequence as a single-token argument
            let mut k = j + 1;
            if chars[k].is_alphabetic() {
                while k < chars.len() && chars[k].is_alphabetic() {
                    k += 1;
                }
            } else {
                k += 1;
            }
            args.push(chars[j..k].iter().collect());
            j = k;
        } else {
            args.push(chars[j].to_string());
            j += 1;
        }
    }
    (args, j)
}

/// Remove text-mode helpers that math renderers reject.
fn clean_math(math: &str) -> String {
    if !math.contains('\\') {
        return math.to_string();
    }
    let mut out = String::with_capacity(math.len());
    let chars: Vec<char> = math.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1].is_alphabetic() {
            let mut end = i + 1;
            while end < chars.len() && chars[end].is_alphabetic() {
                end += 1;
            }
            let name: String = chars[i + 1..end].iter().collect();
            match name.as_str() {
                // \xspace, \protect, \relax, \nobreak have no meaning in math
                "xspace" | "protect" | "relax" | "nobreak" | "allowbreak" | "ignorespaces" => {
                    i = end;
                    // swallow one following space only if it would join words
                    continue;
                }
                // \ensuremath{x} inside math is just {x}
                "ensuremath" => {
                    i = end;
                    continue;
                }
                _ => {
                    out.extend(&chars[i..end]);
                    i = end;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(params: usize, body: &str) -> MacroDef {
        MacroDef {
            num_params: params,
            body: body.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn expands_nested_macros() {
        let mut m = HashMap::new();
        m.insert("R".to_string(), def(0, "\\mathbb{R}"));
        m.insert("norm".to_string(), def(1, "\\lVert #1 \\rVert"));
        assert_eq!(expand_math("\\norm{x} \\in \\R", &m), "\\lVert x \\rVert \\in \\mathbb{R}");
    }

    #[test]
    fn recursion_is_not_expanded() {
        let mut m = HashMap::new();
        m.insert("a".to_string(), def(0, "x\\a"));
        assert_eq!(expand_math("\\a", &m), "x\\a");
    }

    #[test]
    fn programming_macros_are_left_alone() {
        let mut m = HashMap::new();
        m.insert("sm".to_string(), def(0, "\\@ifnextchar\\bgroup{\\sm}{x}"));
        assert_eq!(expand_math("\\sm", &m), "\\sm");
    }

    #[test]
    fn single_token_arguments() {
        let mut m = HashMap::new();
        m.insert("vec".to_string(), def(1, "\\mathbf{#1}"));
        assert_eq!(expand_math("\\vec x + \\vec\\alpha", &m), "\\mathbf{x} + \\mathbf{\\alpha}");
    }

    #[test]
    fn control_word_does_not_fuse_with_following_letter() {
        let mut m = HashMap::new();
        m.insert("eps".to_string(), def(0, "\\varepsilon"));
        assert_eq!(expand_math("\\eps x", &m), "\\varepsilon x");
    }

    #[test]
    fn strips_text_helpers() {
        let m = HashMap::new();
        assert_eq!(expand_math("\\ensuremath{\\alpha}\\xspace", &m), "{\\alpha}");
    }
}
