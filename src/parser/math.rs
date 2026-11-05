//! Clean-up of math content for Markdown math renderers (KaTeX/MathJax).
//!
//! Document macros inside math are expanded by the TeX token engine
//! (`Parser::expand_math_with_engine` in tex.rs); this module removes
//! text-mode helpers that are meaningless (or errors) inside a formula.

/// Remove text-mode helpers that math renderers reject.
pub(super) fn clean_math(math: &str) -> String {
    let math = &math.replace("-NoValue-", "");
    if !math.contains('\\') && !math.contains('$') {
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
                    continue;
                }
                // \ensuremath{x} inside math is just {x}
                "ensuremath" => {
                    i = end;
                    continue;
                }
                // Text boxes holding nested $...$: math stays math, text
                // becomes \text (a nested `$` would end the Markdown span)
                "hbox" | "mbox" | "text" | "textrm" | "textnormal" | "textup" | "textit"
                | "textbf" | "textsf" | "texttt" | "fbox" | "makebox" => {
                    let mut j = end;
                    while j < chars.len() && chars[j] == ' ' {
                        j += 1;
                    }
                    if j < chars.len() && chars[j] == '{' {
                        if let Some(close) = matching_brace(&chars, j) {
                            let inner: String = chars[j + 1..close].iter().collect();
                            if inner.contains('$') {
                                let wrapper = match name.as_str() {
                                    "hbox" | "mbox" | "fbox" | "makebox" | "textnormal" | "textup" => "text",
                                    other => other,
                                };
                                out.push_str(&split_text_math(&inner, wrapper));
                                i = close + 1;
                                continue;
                            }
                        }
                    }
                    out.extend(&chars[i..end]);
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
        if chars[i] == '\\' && i + 1 < chars.len() {
            out.push(chars[i]);
            out.push(chars[i + 1]);
            i += 2;
            continue;
        }
        // A bare `$` cannot appear inside Markdown math
        if chars[i] == '$' {
            out.push_str("\\$");
            i += 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn matching_brace(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0;
    let mut k = open;
    while k < chars.len() {
        match chars[k] {
            '\\' => k += 1,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(k);
                }
            }
            _ => {}
        }
        k += 1;
    }
    None
}

/// `a $x$ b` (text with nested math) -> `\text{a }x\text{ b}`.
fn split_text_math(inner: &str, wrapper: &str) -> String {
    let mut out = String::new();
    let mut in_math = false;
    let mut current = String::new();
    let mut chars = inner.chars().peekable();
    let flush = |out: &mut String, current: &mut String, in_math: bool| {
        if current.is_empty() {
            return;
        }
        if in_math {
            out.push('{');
            out.push_str(&clean_math(current));
            out.push('}');
        } else {
            out.push_str(&format!("\\{}{{{}}}", wrapper, current));
        }
        current.clear();
    };
    while let Some(c) = chars.next() {
        if c == '\\' {
            current.push(c);
            if let Some(n) = chars.next() {
                current.push(n);
            }
            continue;
        }
        if c == '$' {
            flush(&mut out, &mut current, in_math);
            in_math = !in_math;
            continue;
        }
        current.push(c);
    }
    flush(&mut out, &mut current, in_math);
    out
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
    fn expands_nested_macros() {
        let md = convert("\\newcommand{\\R}{\\mathbb{R}}\\newcommand{\\norm}[1]{\\lVert #1 \\rVert}$\\norm{x} \\in \\R$");
        assert!(md.contains("$\\lVert x \\rVert \\in \\mathbb{R}$"), "got: {}", md);
    }

    #[test]
    fn recursion_terminates() {
        let md = convert("\\newcommand{\\a}{x\\a}$\\a$");
        assert!(md.len() < 100_000, "got {} bytes", md.len());
    }

    #[test]
    fn ifnextchar_macros_expand() {
        let md = convert(
            "\\makeatletter\\def\\prd#1{\\@ifnextchar\\bgroup{\\prd@parens{#1}}{\\prod_{#1}}}\
             \\def\\prd@parens#1{\\prod_{(#1)}\\@ifnextchar\\bgroup{\\prd@parens}{}}\\makeatother\
             $\\prd{x:A}{y:B} C$ and $\\prd{z} D$",
        );
        // \@ifnextchar skips the space before peeking, as in TeX
        assert!(md.contains("$\\prod_{(x:A)}\\prod_{(y:B)}C$"), "got: {}", md);
        assert!(md.contains("$\\prod_{z}D$"), "got: {}", md);
    }

    #[test]
    fn single_token_arguments() {
        let md = convert("\\newcommand{\\vect}[1]{\\mathbf{#1}}$\\vect x + \\vect\\alpha$");
        assert!(md.contains("\\mathbf{x} + \\mathbf{\\alpha}"), "got: {}", md);
    }

    #[test]
    fn control_word_does_not_fuse_with_following_letter() {
        let md = convert("\\newcommand{\\eps}{\\varepsilon}$\\eps x$ $a\\eps$");
        assert!(md.contains("$\\varepsilon x$"), "got: {}", md);
    }

    #[test]
    fn primes_and_dashes_survive_in_math() {
        let md = convert("\\newcommand{\\f}{f}$\\f''(x) -- y$");
        assert!(md.contains("$f''(x) -- y$"), "got: {}", md);
    }

    #[test]
    fn let_alias_in_math() {
        let md = convert("\\let\\eps\\varepsilon $\\eps$");
        assert!(md.contains("$\\varepsilon$"), "got: {}", md);
    }

    #[test]
    fn nested_dollars_never_reach_output() {
        assert_eq!(super::clean_math("\\hbox{$\\ulcorner$}x"), "{\\ulcorner}x");
        assert_eq!(super::clean_math("\\text{if $x$ holds}"), "\\text{if }{x}\\text{ holds}");
        assert_eq!(super::clean_math("a$b"), "a\\$b");
    }

    #[test]
    fn strips_text_helpers() {
        assert_eq!(super::clean_math("\\ensuremath{\\alpha}\\xspace"), "{\\alpha}");
    }
}
