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

// --- Floats, labels, programming layers ------------------------------------------

#[test]
fn declared_floats_are_numbered_with_captions() {
    let doc = "\\documentclass{book}\\usepackage{float}\\newfloat{program}{tbp}{lop}[chapter]\\floatname{program}{Program}\n\\begin{document}\\chapter{One}\n\\begin{program}\\begin{verbatim}\nx = 1\n\\end{verbatim}\n\\caption{Assign}\\label{p:a}\\end{program}\nSee \\ref{p:a}.\\end{document}";
    has(doc, "**Program 1.1:** Assign");
    has(doc, "See 1.1.");
    has(doc, "x = 1");
}

#[test]
fn minted_listing_float() {
    let doc = "\\begin{listing}\\begin{minted}{python}\nprint(1)\n\\end{minted}\n\\caption{Hello}\\label{lst:h}\\end{listing} \\autoref{lst:h}";
    has(doc, "```python\nprint(1)\n```");
    has(doc, "**Listing 1:** Hello");
    has(doc, "Listing 1");
    lacks(doc, "\\begin{listing}");
}

#[test]
fn declare_floating_environment_name_option() {
    let doc = "\\DeclareFloatingEnvironment[name=Snippet]{snip}\\begin{snip}Body.\\caption{Cap}\\end{snip}";
    has(doc, "**Snippet 1:** Cap");
}

#[test]
fn label_in_running_text_refers_to_section() {
    has("\\section{A}\\section{B}Text \\label{here}. See \\ref{here}.", "See 2.");
}

#[test]
fn label_inside_theorem_body_refers_to_theorem() {
    has("\\newtheorem{thm}{Theorem}\\begin{thm}Claim \\label{t}.\\end{thm}\\begin{thm}Two\\end{thm} \\ref{t}", " 1");
}

#[test]
fn user_environment_inside_raw_capture_is_not_expanded() {
    // `lined` opens a tabular in its begin code; inside a raw-captured
    // environment its \end must stay as written
    let doc = "\\newenvironment{lined}{\\begin{tabular}{l}}{\\end{tabular}}\\begin{mysteryfloat}\\begin{lined}x\\end{lined}\\end{mysteryfloat}\n\nAfter.";
    has(doc, "\\end{lined}");
    has(doc, "After.");
}

#[test]
fn trivlist_is_transparent() {
    has("\\begin{trivlist}\\item Inside.\\end{trivlist}", "Inside.");
    lacks("\\begin{trivlist}\\item Inside.\\end{trivlist}", "trivlist");
}

#[test]
fn new_command_copy_is_let() {
    has("\\newcommand{\\foo}{Batman}\\NewCommandCopy{\\copied}{\\foo}\\renewcommand{\\foo}{Robin}\\copied{} \\foo", "Batman Robin");
}

#[test]
fn expl3_definitions_and_constants() {
    let doc = "\\ExplSyntaxOn\\cs_set_eq:NN \\bs \\c_backslash_str\\cs_new:Npn \\my_greet:n #1 { Hello~#1! }\\cs_new:Nn \\my_twice:n { #1#1 }\\NewDocumentCommand{\\greet}{m}{\\my_greet:n {#1}}\\NewDocumentCommand{\\twice}{m}{\\my_twice:n {#1}}\\ExplSyntaxOff\nA \\bs{} B \\greet{World} \\twice{ab}.";
    has(doc, "A \\\\ B Hello World! abab.");
}

#[test]
fn expl3_private_names_lex_as_one_command() {
    let doc = "\\ProvidesExplFile{x}{d}{v}{desc}\n\\cs_new:Npn \\__mod_get:n #1 { \\str_case:nn {#1} { {g}{global} } }\nText.";
    has(doc, "Text.");
    lacks(doc, "mod");
}

#[test]
fn declared_si_units() {
    has("\\DeclareSIUnit\\parsec{pc}\\qty{3}{\\kilo\\parsec}", "3\u{a0}kpc");
}

#[test]
fn xparse_verbatim_argument_with_delimiter() {
    has("\\NewDocumentCommand{\\code}{v}{\\texttt{#1}}\\code|a$b$c| after", "a$b$c");
    has("\\NewDocumentCommand{\\code}{v}{[#1]}\\code|x| after", "after");
}

#[test]
fn csquotes_textquote() {
    has("\\textquote[Knuth]{Premature optimisation}", "\"Premature optimisation\" (Knuth)");
}

#[test]
fn expandafter_let_csname() {
    let doc = "\\makeatletter\\def\\orig@x{Original}\\def\\x{Changed}\\expandafter\\let\\csname x\\expandafter\\endcsname\\csname orig@x\\endcsname\\makeatother\n\\x";
    has(doc, "Original");
    lacks(doc, "Changed");
    has("\\expandafter\\def\\csname my name\\endcsname{Made}\\csname my name\\endcsname", "Made");
}

#[test]
fn german_babel_shorthands() {
    let doc = "\\documentclass{article}\\usepackage[ngerman]{babel}\\begin{document}Gr\"o\"se und Stra\"se, \"`Zitat\"' und Zucker\"-b\"acker.\\end{document}";
    has(doc, "Größe und Straße, \u{201e}Zitat\u{201c} und Zuckerbäcker.");
    // without German, a double quote stays a double quote
    has("Say \"a\" here.", "\"a\"");
}

#[test]
fn defined_shorthands() {
    has("\\newcommand{\\tok}[1]{(#1)}\\useshorthands{!}\\defineshorthand{!!}{\\tok}Some !!{word} here.", "Some (word) here.");
}

#[test]
fn branch_arguments_after_line_end() {
    has("\\NewDocumentCommand{\\a}{s}{\\IfBooleanTF {#1}\n    {Y}{N}}\\a* \\a.", "Y N.");
}

#[test]
fn indentation_after_comment_is_not_a_space() {
    has("\\newcommand{\\x}{a%\n    b}\\x", "ab");
}

#[test]
fn xparse_braced_token_argument() {
    let doc = "\\DeclareDocumentCommand \\eq { t{/} o o } {\\IfNoValueTF {#3}{=}{\\IfBooleanTF{#1}{#2 N #3}{#2 E #3}}}\\eq[A][B] and \\eq/[A][B].";
    has(doc, "A E B and A N B.");
}

#[test]
fn newcommand_body_after_line_break() {
    has("\\newcommand{\\ia}[1]\n  {(#1)}\\ia{x}", "(x)");
}

#[test]
fn etoolbox_and_kernel_list_loops() {
    has("\\renewcommand*{\\do}[1]{(#1)}\\docsvlist{a, b ,c}", "(a)(b)(c)");
    has("\\makeatletter\\@for\\it:=a,b\\do{[\\it]}", "\\[a\\]\\[b\\]");
    has("\\listadd{\\L}{one}\\listadd{\\L}{two}\\renewcommand*{\\do}[1]{<#1>}\\dolistloop{\\L}", "one>");
    has("\\newcommand{\\iftag}[3]{\\def\\iftagtext{#3}\\def\\do##1{\\iftoggle{##1}{\\def\\iftagtext{#2}}{}}\\docsvlist{#1}\\iftagtext}\\newtoggle{X}\\toggletrue{X}\\iftag{X}{yes}{no} \\iftag{Y}{yes}{no}", "yes no");
}

#[test]
fn xparse_nested_optional_arguments() {
    has("\\NewDocumentCommand{\\q}{o o}{\\IfNoValueTF{#2}{#1}{#1:#2}}\\q[a][\\q[b][c]]", "a:b:c");
}

#[test]
fn prooftree_is_kept_as_latex() {
    has("\\begin{prooftree}\\AxiomC{A}\\UnaryInfC{B}\\end{prooftree}", "```latex\n\\begin{prooftree}");
}

#[test]
fn pifont_dingbats() {
    has("Yes \\ding{51}, no \\ding{55}, one \\ding{172}, arrow \\ding{220}.", "Yes ✓, no ✗, one ①, arrow ➜.");
}

#[test]
fn margin_and_todo_notes_become_footnotes() {
    let out = md("A\\marginpar{side} B\\todo[inline]{fix}.");
    assert!(out.contains("A[^1] B[^2]."), "{}", out);
    assert!(out.contains("[^1]: side"), "{}", out);
    assert!(out.contains("[^2]: TODO: fix"), "{}", out);
}

#[test]
fn tabbing_lines_and_cells() {
    let out = md("\\begin{tabbing}Name \\= Value \\kill\nAlpha \\> 1 \\\\\nBeta \\> 22\n\\end{tabbing}");
    assert!(out.contains("Alpha\u{2003}1  \nBeta\u{2003}22"), "{:?}", out);
}

#[test]
fn beamer_frame_command_and_subtitle() {
    let doc = "\\documentclass{beamer}\\title{T}\\subtitle{Sub}\\begin{document}\\frame{\\titlepage}\\begin{frame}\\frametitle{First}Hello.\\end{frame}\\end{document}";
    has(doc, "# T\n\n**Sub**");
    has(doc, "### First\n\nHello.");
}

#[test]
fn email_links_and_placeholders() {
    has("Write to \\email{a@b.org}.", "[a@b.org](mailto:a@b.org)");
    has("\\lipsum[1]", "Lorem ipsum dolor sit amet");
}

#[test]
fn transparent_preview_and_letter() {
    has("\\begin{preview}Shown.\\end{preview}", "Shown.");
    let doc = "\\begin{letter}{Jane Doe\\\\Street}\\opening{Dear Jane,}\n\nBody text.\n\n\\closing{Yours,}\\end{letter}";
    has(doc, "Dear Jane,");
    has(doc, "Body text.");
    has(doc, "Yours,");
}

// --- Plain TeX ----------------------------------------------------------------------

#[test]
fn plain_tex_count_registers_and_loops() {
    let doc = "\\newcount\\n \\n=0\n\\loop \\advance\\n by 1 [\\the\\n]\\ifnum\\n<3 \\repeat. \\def\\cols{\\the\\n}\\if 3\\cols three\\else other\\fi";
    has(doc, "[1] [2] [3].");
    has(doc, "three");
    lacks(doc, "other");
}

#[test]
fn plain_tex_set_up_produces_nothing() {
    let doc = "\\font\\big=cmbx10 scaled \\magstep2 \\font\\x=cmr10 at 12pt \\textfont0=\\big \\catcode`\\^=12 \\chardef\\{=`\\{ \\output={\\plainoutput} \\footline={\\hss\\folio} Visible.";
    has(doc, "Visible.");
    lacks(doc, "scaled");
    lacks(doc, "=12");
    lacks(doc, "cmbx");
}

#[test]
fn plain_tex_full_width_boxes_are_lines() {
    has("\\hbox to \\hsize{first}\\hbox to \\hsize{second}", "first  \nsecond");
    has("\\hbox to 1em{\\hss.\\hss}x", ". x");
}

#[test]
fn active_character_definitions_are_consumed() {
    // \def%{...} with % made active must not swallow the next line
    has("\\def\\x{X}\n\\gdef\\y{\\def%{pct}}\n\\x", "X");
    lacks("\\def\\x{X}\n\\gdef\\y{\\def%{pct}}\n\\x", "pct");
}

#[test]
fn space_factor_command_is_kept_out_of_arguments() {
    has("\\def\\mk#1{[#1]}Begin with G\\@. Next \\mk\\@ x.", "Begin with G. Next");
}

#[test]
fn redefined_builtin_environment_keeps_its_parser() {
    // tufte-style redefinition of quote, wrapped by another environment
    let doc = "\\makeatletter\\renewenvironment{quote}{\\list{}{}\\item\\relax}{\\endlist}\\makeatother\\newenvironment{docspec}{\\begin{quote}}{\\end{quote}}\\begin{docspec}Inside.\\end{docspec}\n\n\\section{After}\nText.";
    has(doc, "> Inside.\n\n## After");
}
