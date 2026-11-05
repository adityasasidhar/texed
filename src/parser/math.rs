//! Clean-up of math content for Markdown math renderers (KaTeX/MathJax).
//!
//! Document macros inside math are expanded by the TeX token engine
//! (`Parser::expand_math_with_engine` in tex.rs); this module removes
//! text-mode helpers that are meaningless (or errors) inside a formula.

/// Remove text-mode helpers that math renderers reject.
pub(super) fn clean_math(math: &str) -> String {
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
    fn strips_text_helpers() {
        assert_eq!(super::clean_math("\\ensuremath{\\alpha}\\xspace"), "{\\alpha}");
    }
}
