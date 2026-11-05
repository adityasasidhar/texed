//! Feature tests: one focused check per supported LaTeX construct, run
//! through the public library API.

fn md(latex: &str) -> String {
    texed::convert_with(
        latex,
        &texed::Options {
            no_frontmatter: true,
            ..Default::default()
        },
    )
    .expect("conversion succeeds")
    .markdown
}

fn has(latex: &str, expected: &str) {
    let out = md(latex);
    assert!(out.contains(expected), "expected {:?} in output of {:?}\n--- got ---\n{}", expected, latex, out);
}

fn lacks(latex: &str, unexpected: &str) {
    let out = md(latex);
    assert!(!out.contains(unexpected), "did not expect {:?} in output of {:?}\n--- got ---\n{}", unexpected, latex, out);
}

// --- Document structure -----------------------------------------------------

#[test]
fn preamble_is_not_typeset() {
    let doc = "\\documentclass{article}\\usepackage{xcolor}\\setlength{\\parindent}{0pt}stray\n\\begin{document}Body.\\end{document}After end.";
    has(doc, "Body.");
    lacks(doc, "stray");
    lacks(doc, "0pt");
    lacks(doc, "After end");
}

#[test]
fn starred_sections_are_unnumbered() {
    has("\\section{A}\\section*{B}\\section{C}\\label{c} \\ref{c}", "2");
    lacks("\\section{A}\\section*{B}\\section{C}\\label{c} See \\ref{c}.", "See 3.");
}

#[test]
fn appendix_numbering() {
    has("\\section{A}\\appendix\\section{X}\\label{x}\\subsection{Y}\\label{y} \\ref{x} \\ref{y}", "A A.1");
}

#[test]
fn book_numbering_per_chapter() {
    has(
        "\\documentclass{book}\\begin{document}\\chapter{C}\\chapter{D}\\begin{figure}\\caption{F}\\label{f}\\end{figure}\\begin{equation}x\\label{e}\\end{equation}\\ref{f} \\eqref{e}\\end{document}",
        "2.1 (2.1)",
    );
}

#[test]
fn documentstyle_latex209() {
    has("\\documentstyle[12pt]{article}\\begin{document}Old style.\\end{document}", "Old style.");
}

// --- Text, fonts, characters ---------------------------------------------------

#[test]
fn old_style_font_declarations() {
    has("Normal {\\bf bold} and {\\it italic} text", "**bold**");
    has("Normal {\\bf bold} and {\\it italic} text", "*italic*");
    has("start \\bf rest of paragraph", "start **rest of paragraph**");
    has("{\\tt code}", "`code`");
}

#[test]
fn declaration_environments() {
    has("\\begin{em}emphasized\\end{em}", "*emphasized*");
    has("\\begin{bf}bold\\end{bf}", "**bold**");
}

#[test]
fn nested_emphasis_toggles() {
    has("\\emph{outer \\emph{inner} outer}", "*outer* inner *outer*");
}

#[test]
fn accents_and_symbols() {
    has("caf\\'e na\\\"ive \\c{c}a \\ss{} \\AA{} \\o", "café naïve ça ß Å ø");
    has("\\textbackslash{} \\textasciitilde{} \\textless{}", "\\\\ \\~ \\<");
}

#[test]
fn tex_logos() {
    has("\\LaTeX, \\AmS-\\TeX, \\XeLaTeX", "LaTeX, AMS-TeX, XeLaTeX");
}

#[test]
fn literal_brackets_are_kept() {
    has("See [1] and [note].", "See [1] and [note].");
}

#[test]
fn urls_are_verbatim() {
    has("\\url{http://example.com/a%20b#frag}", "<http://example.com/a%20b#frag>");
    has("\\href{http://x.org/~u_v}{link}", "[link](http://x.org/~u_v)");
}

// --- Macros and TeX programming ---------------------------------------------------

#[test]
fn newcommand_with_optional_argument() {
    has("\\newcommand{\\greet}[2][Hello]{#1, #2!}\\greet{World} \\greet[Hi]{You}", "Hello, World! Hi, You!");
}

#[test]
fn def_with_delimited_parameters() {
    has("\\def\\pair(#1,#2){<#1|#2>}\\pair(a,b)", "\\<a|b>");
    has("\\def\\dot#1.{[#1]}\\dot hello world. after", "[hello world] after");
}

#[test]
fn single_token_arguments() {
    has("\\newcommand{\\twice}[1]{#1#1}\\twice x", "xx");
}

#[test]
fn conditionals() {
    has("\\iffalse hidden \\fi shown", "shown");
    lacks("\\iffalse hidden \\fi shown", "hidden");
    has("\\newif\\ifdraft \\drafttrue \\ifdraft DRAFT\\else FINAL\\fi", "DRAFT");
    has("\\newif\\ifdraft \\ifdraft DRAFT\\else FINAL\\fi", "FINAL");
    has("\\ifnum 3>2 yes\\else no\\fi", "yes");
    has("\\ifcase 2 zero\\or one\\or two\\else many\\fi", "two");
    has("\\newtoggle{t}\\toggletrue{t}\\iftoggle{t}{ON}{OFF}", "ON");
}

#[test]
fn let_captures_meaning() {
    has("\\let\\oldemph\\emph \\renewcommand{\\emph}[1]{[\\oldemph{#1}]} \\emph{x}", "[*x*]");
}

#[test]
fn makeatletter_macros() {
    has("\\makeatletter\\def\\@hidden{AT}\\newcommand{\\show}{\\@hidden}\\makeatother \\show", "AT");
}

#[test]
fn ifnextchar_optional_arguments() {
    has(
        "\\makeatletter\\def\\opt{\\@ifnextchar[{\\opt@a}{\\opt@b}}\\def\\opt@a[#1]{A:#1}\\def\\opt@b{B}\\makeatother \\opt[x] \\opt",
        "A:x B",
    );
}

#[test]
fn macros_expand_inside_math() {
    has("\\newcommand{\\R}{\\mathbb{R}}$x \\in \\R^n$", "$x \\in \\mathbb{R}^n$");
    has("\\DeclareMathOperator{\\rank}{rank}$\\rank A$", "\\operatorname{rank}");
}

#[test]
fn runaway_macros_terminate() {
    let out = md("\\newcommand{\\a}{x\\a}\\a \\def\\b{\\b\\b}\\b");
    assert!(out.len() < 200_000, "output too large: {}", out.len());
}

// --- Math -------------------------------------------------------------------------

#[test]
fn math_environments_keep_raw_math() {
    has("\\begin{equation}f''(x) = a -- b\\end{equation}", "f''(x) = a -- b");
}

#[test]
fn align_rows_numbered_individually() {
    let doc = "\\begin{align} a &= 1 \\label{r1}\\\\ b &= 2 \\nonumber \\\\ c &= 3 \\label{r3}\\end{align} \\eqref{r1} \\eqref{r3}";
    has(doc, "(1) (2)");
    has(doc, "\\begin{aligned}");
}

#[test]
fn unterminated_dollar_does_not_eat_document() {
    has("Cost $5 today.\n\nNext paragraph.", "Next paragraph.");
}

// --- References, citations, footnotes ------------------------------------------------

#[test]
fn cleveref_names_and_lists() {
    let doc = "\\section{A}\\label{a}\\section{B}\\label{b}\\begin{figure}\\caption{F}\\label{f}\\end{figure}\\begin{equation}x\\label{e}\\end{equation} \\cref{a,b} \\Cref{f} \\cref{e}";
    has(doc, "sections 1 and 2 Figure 1 eq. (1)");
}

#[test]
fn forward_references_resolve() {
    has("See \\ref{later}. \\section{Later}\\label{later}", "See 1.");
}

#[test]
fn theorem_references_use_their_name() {
    has("\\newtheorem{lemma}{Lemma}\\begin{lemma}\\label{l}X\\end{lemma} \\cref{l}", "lemma 1");
}

#[test]
fn footnotes() {
    has("Text\\footnote{A note.} more.", "Text[^1] more.");
    has("Text\\footnote{A note.} more.", "[^1]: A note.");
}

#[test]
fn thebibliography_and_cite() {
    let doc = "As \\cite{k} shows.\\begin{thebibliography}{9}\\bibitem{k} Knuth. TeXbook.\\end{thebibliography}";
    has(doc, "As [1] shows.");
    has(doc, "[1] Knuth. TeXbook.");
}

// --- Lists ------------------------------------------------------------------------

#[test]
fn custom_item_labels_kept() {
    has("\\begin{itemize}\\item[--] dash\\end{itemize}", "- – dash");
}

#[test]
fn enumerate_start_from_setcounter() {
    has("\\begin{enumerate}\\setcounter{enumi}{4}\\item five\\end{enumerate}", "5. five");
}

#[test]
fn adjacent_lists_do_not_merge() {
    has("\\begin{itemize}\\item a\\end{itemize}\\begin{itemize}\\item b\\end{itemize}", "<!-- -->");
}

#[test]
fn description_lists() {
    has("\\begin{description}\\item[Term] Definition.\\end{description}", "- **Term**: Definition.");
}

// --- Floats -------------------------------------------------------------------------

#[test]
fn subfigures() {
    let doc = "\\begin{figure}\\begin{subfigure}{.5\\textwidth}\\includegraphics{a.png}\\caption{Left}\\label{fa}\\end{subfigure}\\caption{Both}\\label{fb}\\end{figure} \\ref{fa} \\ref{fb}";
    has(doc, "![(a) Left](a.png)");
    has(doc, "1a 1");
}

#[test]
fn table_with_caption_and_booktabs() {
    let doc = "\\begin{table}\\caption{Results}\\label{t}\\begin{tabular}{lr}\\toprule A & B \\\\\\midrule 1 & 2 \\\\\\bottomrule\\end{tabular}\\end{table} \\cref{t}";
    has(doc, "**Table 1:** Results");
    has(doc, "| A   |   B |");
    has(doc, "table 1");
}

#[test]
fn nested_tabular_stays_in_cell() {
    has("\\begin{tabular}{ll} a & \\begin{tabular}{c} x \\\\ y \\end{tabular} \\\\ b & c \\end{tabular}", "x<br>y");
}

#[test]
fn longtable_heads_not_duplicated() {
    let out = md("\\begin{longtable}{ll} H1 & H2 \\\\\\endfirsthead H1 & H2 \\\\\\endhead a & b \\\\\\end{longtable}");
    assert_eq!(out.matches("H1").count(), 1, "got: {}", out);
}

#[test]
fn tikz_preserved_verbatim() {
    has("\\begin{tikzpicture}\\draw (0,0)--(1,1);\\end{tikzpicture}", "\\draw (0,0)--(1,1);");
}

#[test]
fn algorithms() {
    let doc = "\\begin{algorithm}\\caption{Sum}\\begin{algorithmic}[1]\\State $s\\gets 0$\\For{$i=1$ to $n$}\\State $s \\gets s+i$\\EndFor\\State \\Return $s$\\end{algorithmic}\\end{algorithm}";
    has(doc, "**Algorithm 1** Sum");
    has(doc, "**for** $i=1$ to $n$ **do**");
    has(doc, "**end for**");
}

// --- Packages ------------------------------------------------------------------------

#[test]
fn siunitx() {
    has("\\SI{9.81}{\\meter\\per\\second\\squared}", "9.81\u{a0}m/s²");
    has("\\num{1.5e-3}", "1.5 × 10⁻³");
}

#[test]
fn acronyms() {
    has(
        "\\newacronym{cpu}{CPU}{Central Processing Unit}\\gls{cpu} and \\gls{cpu}",
        "Central Processing Unit (CPU) and CPU",
    );
}

#[test]
fn beamer_frames() {
    let doc = "\\begin{frame}{Slide}\\begin{itemize}\\item<2-> point\\end{itemize}\\end{frame}";
    has(doc, "### Slide");
    has(doc, "- point");
    lacks(doc, "<2->");
}

#[test]
fn listings_and_minted() {
    has("\\begin{lstlisting}[language=Python]\nx = 1 # c\n\\end{lstlisting}", "```python\nx = 1 # c\n```");
    has("\\mintinline{rust}{let x = {1};}", "`let x = {1};`");
}

#[test]
fn keywords() {
    has("\\keywords{alpha, beta}", "**Keywords:** alpha, beta");
}

#[test]
fn unknown_command_keeps_argument_text() {
    has("A \\mysterymarkup{important} word.", "A important word.");
}

#[test]
fn unknown_environment_is_fenced() {
    has("\\begin{mystery}x\\end{mystery}", "```latex\n\\begin{mystery}x\\end{mystery}\n```");
}

// --- Input robustness ------------------------------------------------------------------

#[test]
fn crlf_and_cr_line_endings() {
    has("a\r\n\r\nb\rc", "a\n\nb c");
}

#[test]
fn deep_nesting_is_safe() {
    let doc = format!("{}x{}", "{".repeat(50_000), "}".repeat(50_000));
    has(&doc, "x");
}
