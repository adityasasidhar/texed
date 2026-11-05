//! Robustness tests: conversion must be total (never panic, never hang,
//! never blow up output) on arbitrary and adversarial input.

use std::time::{Duration, Instant};

/// Small deterministic PRNG (xorshift64*), so failures are reproducible
/// without pulling in a dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// LaTeX fragments chosen to exercise every lexer/parser path, including
/// deliberately unbalanced and half-finished constructs.
const FRAGMENTS: &[&str] = &[
    "\\begin{", "\\end{", "\\begin{itemize}", "\\end{itemize}", "\\begin{enumerate}",
    "\\end{enumerate}", "\\begin{description}", "\\end{description}", "\\item", "\\item[",
    "\\begin{tabular}{|l|c|}", "\\end{tabular}", "&", "\\\\", "\\\\[2pt]", "\\hline",
    "\\multicolumn{2}{c}{", "\\begin{table}", "\\end{table}", "\\begin{figure}",
    "\\end{figure}", "\\caption{", "\\label{", "\\ref{", "\\eqref{", "\\cite{", "\\cite[p.~3]{",
    "{", "}", "[", "]", "$", "$$", "\\[", "\\]", "\\(", "\\)", "%", "\n", "\n\n", " ", "~",
    "\\section{", "\\section*{", "\\chapter{", "\\paragraph{", "\\textbf{", "\\emph{",
    "\\footnote{", "\\href{", "\\url{", "\\verb|", "\\verb", "|", "\\newcommand{\\x}{",
    "\\newcommand{\\y}[1]{#1#1}", "\\x", "\\y{", "\\def\\z#1{", "\\z", "\\renewcommand",
    "\\newenvironment{e}{\\begin{quote}}{\\end{quote}}", "\\begin{e}", "\\end{e}",
    "\\begin{equation}", "\\end{equation}", "\\begin{align}", "\\end{align}",
    "\\begin{verbatim}", "\\end{verbatim}", "\\begin{lstlisting}", "\\end{lstlisting}",
    "\\begin{theorem}", "\\end{theorem}", "\\begin{proof}", "\\end{proof}",
    "\\input{", "\\include{x}", "\\usepackage{", "\\title{", "\\author{", "\\maketitle",
    "\\'", "\\\"", "\\c{", "\\~", "\\^", "\\`", "\\ss", "\\&", "\\%", "\\$", "\\#", "\\_",
    "\\{", "\\}", "\\", "#", "^", "_", "*", "`", "'", "''", "``", "--", "---", "<", ">",
    "\\iffalse", "\\fi", "\\else", "\\ifx", "\\newif\\iffoo", "\\footrue", "\\iffoo",
    "\\makeatletter", "\\makeatother", "\\@tempa", "\\let\\a\\b", "\\let", "\\csname",
    "\\endcsname", "\\appendix", "\\setcounter{", "\\begin{thebibliography}{9}",
    "\\bibitem{", "\\end{thebibliography}", "\\begin{document}", "\\end{document}",
    "text", "word ", "é", "日本", "\u{0}", "\r", "\t", "\\par", "\\noindent",
];

fn convert(input: &str) -> String {
    match texed::convert_with(
        input,
        &texed::Options {
            no_frontmatter: false,
            ..Default::default()
        },
    ) {
        Ok(c) => c.markdown,
        Err(e) => panic!("conversion failed on {:?}: {}", input, e),
    }
}

fn check_total(input: &str) {
    let start = Instant::now();
    let output = convert(input);
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(5),
        "too slow ({:?}) on input {:?}",
        elapsed,
        input
    );
    assert!(
        output.len() <= 64 * input.len() + 4096,
        "output blew up ({} bytes from {} input bytes): {:?}",
        output.len(),
        input.len(),
        input
    );
}

#[test]
fn random_fragment_soup_never_fails() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..3000 {
        let len = 1 + rng.below(40);
        let mut input = String::new();
        for _ in 0..len {
            input.push_str(FRAGMENTS[rng.below(FRAGMENTS.len())]);
        }
        check_total(&input);
    }
}

#[test]
fn random_bytes_never_fail() {
    let mut rng = Rng(42);
    let alphabet: Vec<char> = "\\{}[]$%&#^_~ \n\tabcxyz0123'`\"-*|<>@=.,;:!()é".chars().collect();
    for _ in 0..2000 {
        let len = rng.below(200);
        let input: String = (0..len).map(|_| alphabet[rng.below(alphabet.len())]).collect();
        check_total(&input);
    }
}

#[test]
fn every_prefix_of_a_real_document_converts() {
    let doc = include_str!("fixtures/kitchen_sink.tex");
    let chars: Vec<char> = doc.chars().collect();
    let step = (chars.len() / 400).max(1);
    let mut i = 0;
    while i <= chars.len() {
        let prefix: String = chars[..i].iter().collect();
        check_total(&prefix);
        i += step;
    }
}

#[test]
fn deep_nesting_does_not_overflow() {
    for (open, close) in [
        ("{", "}"),
        ("\\textbf{", "}"),
        ("\\emph{", "}"),
        ("\\footnote{", "}"),
        ("\\begin{itemize}\\item ", "\\end{itemize}"),
        ("\\begin{quote}", "\\end{quote}"),
        ("\\begin{center}", "\\end{center}"),
        ("[", "]"),
        ("\\section{", "}"),
    ] {
        let input = format!("{}x{}", open.repeat(20_000), close.repeat(20_000));
        check_total(&input);
    }
}

#[test]
fn self_referential_macros_terminate() {
    for input in [
        "\\newcommand{\\a}{\\a}\\a",
        "\\newcommand{\\a}{\\a\\a}\\a",
        "\\newcommand{\\a}{x\\a}\\a",
        "\\newcommand{\\a}{\\b}\\newcommand{\\b}{\\a}\\a",
        "\\def\\a{\\a}\\a",
        "\\def\\loop{\\loop x}\\loop",
        "\\newcommand{\\a}[1]{\\a{#1#1}}\\a{x}",
        "\\newenvironment{e}{\\begin{e}}{\\end{e}}\\begin{e}x\\end{e}",
        "\\newcommand{\\a}{\\begin{itemize}\\item\\a}\\a",
    ] {
        check_total(input);
    }
}

#[test]
fn huge_repetition_counts_are_capped() {
    check_total("\\begin{tabular}{*{999999999}{c}} a & b \\end{tabular}");
    check_total("\\begin{tabular}{*{99}{*{99}{*{99}{c}}}} a \\end{tabular}");
    check_total("\\setcounter{section}{99999999999999999999}\\section{x}");
    check_total("\\begin{enumerate}[start=18446744073709551615]\\item a\\end{enumerate}");
}

#[test]
fn unterminated_constructs_do_not_swallow_the_document() {
    let md = convert("Price is $5.\n\nNext paragraph stays text.");
    assert!(md.contains("Next paragraph stays text"), "got: {}", md);

    let md = convert("\\label{unclosed\n\nStill here.");
    assert!(md.contains("Still here"), "got: {}", md);

    let md = convert("\\begin{itemize\n\nStill here.");
    assert!(md.contains("Still here"), "got: {}", md);

    let md = convert("\\verb|unterminated\nNext line.");
    assert!(md.contains("Next line"), "got: {}", md);
}
