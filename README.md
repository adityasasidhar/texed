# Texed - LaTeX to Markdown Converter

    (\_/)
    ( •_•)
   / >🦀  Texed

A fast and comprehensive command-line tool written in Rust to convert LaTeX documents to Markdown format.

## Features

- **Converts real documents**: papers, theses, books (tested on the Stacks Project, the HoTT book, the C++ standard draft, LaTeX's own documentation, journal templates and LaTeX 2.09-era sources), beamer slides, package documentation
- **Faithful numbering and cross-references**: sections, `\section*`, `\appendix` (A, B, ...), per-chapter numbering in books, equations (per row in `align`), floats numbered by `\caption`, theorems with shared/scoped counters, subfigures (`1a`), cleveref-style `\cref{a,b}` → "sections 1 and 2"
- **A real TeX macro layer**: `\newcommand`, `\def` with delimited parameters, `\let`, `\newif` and `\if...\else...\fi`, `\@ifnextchar`, `\csname`, xparse, local `.sty` files — and document macros are expanded *inside math* so formulas render in KaTeX/MathJax
- **Packages**: amsmath, amsthm/thmtools, hyperref, cleveref, graphicx, subcaption/subfig, booktabs, longtable, threeparttable, multirow, listings, minted, algorithmicx/algorithm2e, siunitx, glossaries/acronym/acro, natbib/biblatex, beamer, TikZ (kept as code), etoolbox/ifthen, and many more
- **Markdown-safe output**: literal text can never turn into Markdown syntax — verified by round-tripping random special-character text through a CommonMark parser
- **Robust by construction**: never panics, hangs, or blows up on malformed or adversarial input (see [Robustness](#robustness))
- **Any encoding**: UTF-8, UTF-16, Latin-1/Windows-1252; CRLF and classic-Mac CR line endings
- **Fast**: a 400-page book converts in well under a second; usable as a CLI or as a Rust library

## Installation

### From Source

```bash
git clone https://github.com/yourusername/texed.git
cd texed
cargo build --release
```

The binary will be available at `target/release/texed`.

### Using Cargo

```bash
cargo install texed
```

## Usage

### Basic Usage

```bash
texed paper.tex                    # convert to stdout
texed paper.tex -o paper.md        # convert to a file
cat notes.tex | texed              # read from stdin
texed - -o out.md < input.tex      # stdin explicitly
```

### Batch Conversion

```bash
texed ch1.tex ch2.tex ch3.tex      # writes ch1.md, ch2.md, ch3.md next to each input
texed src/*.tex -o build/          # converts everything into build/
```

Batch runs continue past individual failures and exit non-zero if any file failed.

### Checking Conversion Fidelity

`--check` parses without writing output and reports every construct that could
not be converted faithfully — unknown commands or environments, unresolved
cross-references, missing `\input` files, citation keys absent from the
bibliography:

```console
$ texed thesis.tex --check
✓ thesis.tex → checked (2 warnings)
warning: unknown environment `sidewaysfigure*` preserved as raw LaTeX
warning: unresolved reference `\ref{sec:appendix}`
error: --check: 2 conversion warning(s) found
```

Exit code is 0 when clean, 1 when warnings are found — usable as a CI gate.
`--strict` does the same while still writing output.

### Command-Line Options

```
Usage: texed [OPTIONS] [INPUT]...

Arguments:
  [INPUT]...  Input LaTeX file(s); use '-' or omit to read from stdin

Options:
  -o, --output <PATH>        Output file, or directory when converting multiple files ('-' for stdout)
  -f, --force                Overwrite output files if they exist
  -v, --verbose              Show progress, timings, and every conversion warning
  -q, --quiet                Suppress all diagnostics except errors
      --no-frontmatter       Omit the YAML frontmatter block (title/author/date)
      --check                Analyze conversion fidelity without writing output
      --strict               Treat conversion warnings as errors
      --stats                Print document statistics after converting
      --completions <SHELL>  Generate shell completions (bash/zsh/fish/powershell/elvish)
  -h, --help                 Print help
  -V, --version              Print version

Debug:
      --debug-ast            Pretty print the parsed AST
      --debug-tokens         Show tokens from lexer
```

Colored diagnostics are automatic on a terminal and disabled when piped or
when `NO_COLOR` is set.

### Statistics

```console
$ texed paper.tex -o paper.md --stats

  paper.tex
  blocks   5 headings · 30 paragraphs · 2 lists · 1 tables · 1 figures · 2 equations · 2 code · 1 theorems
  inline   5 math · 0 citations · 2 links · 2 footnotes
  size     2.6 KB → 1.8 KB
  time     1.1 ms (lex 0.3 · parse 0.6 · convert 0.2)
```

### Shell Completions

```bash
texed --completions bash > /etc/bash_completion.d/texed
texed --completions zsh  > ~/.zfunc/_texed
texed --completions fish > ~/.config/fish/completions/texed.fish
```

## Robustness

Conversion is *total*: every input produces Markdown.

- Unbalanced braces, unclosed environments, a stray `$`, half-finished `\begin{`, `\verb` without a closing delimiter: each is bounded by TeX's own rules (math cannot span a blank line, `\verb` cannot span a line), so one typo never swallows the rest of the document
- Recursive and self-replicating macros (`\def\a{\a\a}`) are detected and stopped with a warning; expansion budgets scale with document size
- 100,000 levels of nested braces or environments do not overflow the stack
- Constructs that cannot be converted faithfully are reported by `--check` instead of failing the conversion

The test suite fuzzes the converter with random fragment soup, random bytes, every prefix of a real document, deeply nested input and pathological macros.

## Library Usage

```rust
let markdown = texed::convert(r"\section{Hello} $E = mc^2$")?;

let result = texed::convert_with(latex, &texed::Options {
    base_path: Some("paper/".into()), // where \input, \bibliography, .sty files are found
    no_frontmatter: true,
})?;
for warning in &result.warnings { eprintln!("{warning}"); }
```

## Examples

### Simple Text Conversion

**Input (LaTeX):**
```latex
Hello World! This is a simple paragraph.
```

**Output (Markdown):**
```markdown
Hello World! This is a simple paragraph.
```

### Sections and Subsections

**Input (LaTeX):**
```latex
\section{Introduction}
This is the introduction.

\subsection{Background}
Some background information.
```

**Output (Markdown):**
```markdown
## Introduction

This is the introduction.

### Background

Some background information.
```

### Text Formatting

**Input (LaTeX):**
```latex
\textbf{Bold text}, \textit{italic text}, and \texttt{code text}.
```

**Output (Markdown):**
```markdown
**Bold text**, *italic text*, and `code text`.
```

### Lists

**Input (LaTeX):**
```latex
\begin{itemize}
\item First item
\item Second item
\item Third item
\end{itemize}
```

**Output (Markdown):**
```markdown
- First item
- Second item
- Third item
```

### Math Equations

**Input (LaTeX):**
```latex
Inline math: $E = mc^2$

Display math:
$$
\int_0^\infty e^{-x^2} dx = \frac{\sqrt{\pi}}{2}
$$
```

**Output (Markdown):**
```markdown
Inline math: $E = mc^2$

Display math:
$$
\int_0^\infty e^{-x^2} dx = \frac{\sqrt{\pi}}{2}
$$
```

### Tables

**Input (LaTeX):**
```latex
\begin{table}
\caption{Sample Table}
\begin{tabular}{lcc}
Name & Age & City \\
\hline
Alice & 30 & NYC \\
Bob & 25 & LA \\
\end{tabular}
\end{table}
```

**Output (Markdown):**
```markdown
**Table 1:** Sample Table

| Name  | Age | City |
|:------|:---:|:----:|
| Alice | 30  | NYC  |
| Bob   | 25  |  LA  |
```

### Code Blocks

**Input (LaTeX):**
```latex
\begin{verbatim}
fn main() {
    println!("Hello, world!");
}
\end{verbatim}
```

**Output (Markdown):**
````markdown
```
fn main() {
    println!("Hello, world!");
}
```
````

## Supported LaTeX Commands

### Document Structure
- `\part`, `\chapter`, `\section`, `\subsection`, `\subsubsection`, `\paragraph`, `\subparagraph` (starred variants too; H1 is reserved for the document title, so `\section` maps to `##`)
- `\title`, `\author` (incl. `\and`/`\And`/`\AND`), `\date`, `\maketitle`, `\thanks` — emitted as YAML frontmatter plus a title block
- `\label`, `\ref`, `\eqref`, `\autoref`, `\nameref`, `\pageref`, `\cref`/`\Cref`/`\vref`, `\crefrange`, `\namecref`, `\labelcref` — resolved in a second pass, so **forward references work**; cleveref lists are grouped and pluralized (`\cref{a,b,f}` → "sections 1 and 2 and fig. 3")
- `\input`/`\include`/`\import`/`\subfile` (also mid-paragraph or inside a table), `\usepackage`/`\RequirePackage`/`\documentclass` (local `.sty`/`.cls` files next to the document are read, so their macros and theorem definitions apply), `\graphicspath`, `\endinput`
- Preamble handling: material before `\begin{document}` is set-up only; text after `\end{document}` is ignored, exactly as in LaTeX
- Numbering: `\appendix`, per-chapter numbering in book/report classes, `\setcounter`, `\addtocounter`, `\numberwithin`, `\thesection`/`\arabic`/`\roman`/`\alph`
- `\keywords` and `keywords`/`IEEEkeywords` environments
- LaTeX 2.09: `\documentstyle[options]{style}`, `\bf`/`\it`/`\tt`/`\em` declarations anywhere in a group or paragraph, `\begin{em}`-style declaration environments

### Text Formatting
- `\textbf`, `\textit`, `\emph`, `\textsl`, `\texttt`, `\underline`, `\ul`/`\uline`
- `\textsuperscript`, `\textsubscript`, `\textsc`, `\st`/`\sout`, `\hl`
- Declaration groups: `{\bf ...}`, `{\em ...}`, `{\itshape ...}`, `{\scshape ...}`, ...
- `\verb|...|`, `\verb*`, `\lstinline` with arbitrary delimiters
- `\MakeUppercase`/`\MakeLowercase`, `\enquote`, `\mbox`/`\fbox`/`\parbox`/`\makebox`/`\raisebox`/`\resizebox`/`\scalebox` (content preserved)
- `\textcolor`/`\colorbox` (content preserved)

### Special Characters & Typography
- Escapes: `\%`, `\$`, `\&`, `\#`, `\_`, `\{`, `\}`, `\textbackslash`, ...
- Accents: `\'e` → é, `\"o` → ö, `\c{c}` → ç, `\H{o}` → ő, `\v{s}` → š, `\~n` → ñ, and all other standard accents (precomposed Unicode, combining-mark fallback)
- Ligatures/symbols: `\ldots`, `\ss`, `\ae`, `\oe`, `\S`, `\P`, `\dag`, `\copyright`, `\euro`, ...
- Smart quotes: `` ``...'' `` → “...”, `` `...' `` → ‘...’
- TeX dashes: `--` → –, `---` → —; `~` → non-breaking space

### Lists
- `itemize`, `enumerate` (nested, tight rendering, `\item[...]` markers, package options)
- `description` (pandoc-style definition lists)

### Math
- Inline: `$...$` and `\(...\)`; display: `$$...$$` and `\[...\]` — math is captured raw (primes, `--`, quotes are never altered)
- Environments: `equation`, `align`, `gather`, `multline`, `flalign`, `alignat`, `eqnarray`, `split`, `displaymath` (+ starred forms). Multi-line environments are wrapped in `aligned`/`gathered` so KaTeX/MathJax render them
- Every numbered equation (and every numbered `align` row) consumes a number, as in LaTeX; labeled ones get `\tag{n}`; `\nonumber`/`\notag`/`\tag{...}` respected
- **User macros are expanded inside math** with the full macro engine (`\newcommand{\R}{\mathbb{R}}` → `$\mathbb{R}$`, `\@ifnextchar`-based notation macros, `\DeclareMathOperator` → `\operatorname`)

### Tables
- `tabular`, `tabular*`, `tabularx`, `tabulary`, `longtable`, `supertabular`, `tblr`, `NiceTabular`, ... inside `table`/`table*`/`threeparttable` (or standalone); captions numbered (`**Table 1:** ...`)
- Nested tabulars and lists/math inside `p{}` cells are kept in their cell; longtable `\endfirsthead`/`\endhead`/`\endfoot` handled (heads not duplicated); `tablenotes` and source lines kept
- Column specs incl. `p{}/m{}/b{}/X`, `@{}`, `>{}`, `|`, `*{n}{spec}` repetition
- `\hline`, booktabs rules (`\toprule`/`\midrule`/`\bottomrule`/`\cmidrule(lr){2-3}`) filtered correctly
- `\multicolumn` (pads spanned cells), `\multirow`, `\\[len]` spacing
- Output is aligned and padded for raw-text readability

### Figures
- `figure`/`figure*`/`sidewaysfigure`/`wrapfigure`, `\includegraphics` (also inline, `\epsfig`, `\includesvg`), `\caption` with embedded `\label`
- Sub-figures: `subfigure`/`subtable` environments, `\subfloat`, `\subcaptionbox` — rendered as `(a)`, `(b)` images, labels resolve to `1a`; minipages with their own captions are separate figures; TikZ inside figures kept

### Other Environments
- `quote`, `quotation`, `displayquote`, `verse`, `abstract`
- `verbatim` (captured raw — `%`, `$`, `\` inside code are safe), `lstlisting` (with `language=`), `minted`, `alltt`, `comment` (dropped)
- `center`, `flushleft`, `flushright`, `minipage`, `multicols`, `titlepage` (transparent)
- Theorem-like: built-in `theorem`/`lemma`/`proof`/... plus `\newtheorem{...}[shared]{...}[within]` with correct scoped numbering
- `tikzpicture`, `tikzcd`, `circuitikz`, `pspicture`, `forest` preserved verbatim as LaTeX code blocks; asymptote/gnuplot/luacode/pycode as code blocks
- Undefined theorem-like names (`defn`, `prop`, `rmk*`, ...) render as theorems; unknown environments are preserved as fenced LaTeX
- Pseudocode: `algorithm` floats with `algorithmic`/`algpseudocode` (`\State`, `\If`...`\EndIf`, `\Procedure`, ...), classic `\STATE`/`\IF`, and `algorithm2e` (`\If{c}{...}`, `\eIf`, `\ForEach`, `\KwIn`, `\;`) → numbered, indented lines with bold keywords and live math
- beamer: frames become headings (`\frametitle`, subtitles), `block`/`alertblock`, `columns`; overlay specifications (`\item<2->`, `\only`, `\uncover`, `\alt`, `\pause`) are resolved to the final slide

### Macros
- `\newcommand`, `\renewcommand`, `\providecommand`, `\def`, `\DeclareRobustCommand` — braced and unbraced names, `[n]` parameter counts, optional-argument defaults, TeX-style `#1#2` parameter text
- Expansions are re-lexed and re-parsed, so macros producing formatting, math, or whole environments work
- `\newenvironment`/`\renewenvironment` — begin/end definitions expand at use sites (with arguments), composing correctly with built-in environments
- TeX programming: `\def`/`\gdef`/`\edef` with delimited parameters (`\def\foo#1.#2\par{...}`), `\let` (captures the built-in meaning), `\newif` + `\iffoo...\else...\fi`, `\iffalse` blocks, `\ifx`, `\ifnum`, `\ifcase`, `\ifdefined`, etoolbox `\iftoggle`/`\ifbool`/`\ifdef`, ifthen `\ifthenelse`, kernel `\@ifnextchar`/`\@ifstar`/`\@ifundefined`, `\csname...\endcsname`, `\makeatletter`, expl3 names
- xparse `\NewDocumentCommand`, `\DeclareMathOperator`, `\AtBeginDocument` hooks

### Links, Citations, Bibliography
- `\href{url}{text}` (with options), `\url`, `\hyperref[..]{...}`
- `\cite`, `\citep`, `\citet`, `\citeauthor`, `\citeyear`, `\parencite`, `\textcite`, ... with prenotes/postnotes
- **Numeric and author-year styles**: `\bibliographystyle{plainnat/apalike/...}` and `\usepackage[style=authoryear]{biblatex}` switch to natbib-style rendering — `Knuth (1984)`, `(Lamport, 1994; Knuth, 1984)`, `(see Goossens et al., 1993, p. 5)`
- Citations render in a post-parse pass, so numbers stay consistent with the References list even when the style/bibliography is declared at the end of the document
- `.bib` files via `\bibliography`/`\addbibresource` (robust character-level BibTeX parser: single-line entries, nested braces, quoted values, `@string`/`@comment`) → rendered References section
- `thebibliography`/`\bibitem` → numbered References section with resolved `[n]` markers

### Units, Acronyms, Glossaries
- siunitx: `\SI`/`\qty`, `\si`/`\unit`, `\num`, `\ang`, ranges, lists, products → `9.81 m/s²`, `1.5 × 10⁻³`, `30°15′`
- glossaries (`\newacronym`, `\newglossaryentry`, `\gls`, `\Gls`, `\glspl`, `\acrshort`, `\acrlong`, `\acrfull`, ...), acronym (`\acro`, `\ac`, `\acf`, ...), acro (`\DeclareAcronym`) — first use expands to "long (short)", later uses give the short form

### Layout Commands
Print-only commands (`\vspace`, `\centering`, `\newpage`, `\setlength`, font size switches, package configuration like `\hypersetup`/`\lstset`/`\crefname`/`\titleformat`, TeX register assignments like `\parskip 0pt plus 1pt`, ...) are consumed silently — including their arguments — instead of leaking into the output. Unknown commands are dropped with a warning while the text of their arguments is kept.

## Architecture

Texed follows a pipeline inspired by Pandoc:

1. **Decoder** (`src/decode.rs`): bytes in any common encoding → normalized UTF-8
2. **Lexer** (`src/lexer.rs`): tokenizes LaTeX (math and verbatim content captured raw)
3. **Parser** (`src/parser/`): builds an AST, running a TeX-level macro engine (`tex.rs`) and package support modules (tables, figures, algorithms, siunitx, glossaries, ...)
4. **Converter** (`src/converter.rs`): renders Markdown-safe output

This architecture allows for:
- Clean separation of concerns
- Easy debugging with `--debug-tokens` and `--debug-ast`
- Extensibility for adding new LaTeX commands or output formats

## Development

### Running Tests

```bash
cargo test --release
```

Suites: `features` (one case per construct), `markdown_safety` (CommonMark round-trip), `robustness` (fuzzing), `integration_test` (CLI), plus unit tests.

### Running with Debug Output

```bash
cargo run -- input.tex --debug-tokens --debug-ast -v
```

### Building for Release

```bash
cargo build --release
```

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

This project is licensed under the MIT License - see the LICENSE file for details.

## Acknowledgments

- Inspired by [Pandoc](https://pandoc.org/)'s LaTeX reader architecture
- Built with [Clap](https://github.com/clap-rs/clap) for CLI parsing
- Uses [Regex](https://github.com/rust-lang/regex) for pattern matching

## Roadmap

- [x] Support for more LaTeX packages (amsmath, booktabs, hyperref, listings, natbib, ...)
- [x] Bibliography and citation handling (`.bib` files, `thebibliography`, numeric and author-year styles)
- [x] Custom command definitions (`\newcommand`, `\def`, `\newenvironment`, expansion inside math)
- [x] Two-pass cross-reference resolution (forward references)
- [ ] Multiple output formats (HTML, reStructuredText)
- [ ] Configuration file support
- [ ] Better error messages with line/column information
- [x] TeX macro engine (conditionals, `\let`, delimited parameters, `\@ifnextchar`)
- [x] Robustness: total conversion, fuzz-tested
- [ ] Unicode math symbol conversion
