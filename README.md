# Texed - LaTeX to Markdown Converter

    (\_/)
    ( •_•)
   / >🦀  Texed

A fast and comprehensive command-line tool written in Rust to convert LaTeX documents to Markdown format.

## Features

- **Comprehensive LaTeX Support**: Handles sections, subsections, paragraphs, lists, tables, figures, math equations, and more
- **Text Formatting**: Converts bold, italic, underline, superscript, subscript, and code formatting
- **Math Support**: Preserves inline and display math equations
- **Lists**: Supports itemize, enumerate, and description lists
- **Tables**: Converts LaTeX tables to Markdown tables with alignment
- **Figures**: Converts `\includegraphics` to Markdown image syntax
- **Environments**: Handles quote, verbatim, code blocks, and theorem-like environments
- **Fast Performance**: Built in Rust for optimal speed
- **Flexible I/O**: Read from files or stdin, write to files or stdout

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
**Sample Table**

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
- `\label`, `\ref`, `\eqref`, `\autoref`, `\nameref`, `\pageref`, `\cref`/`\Cref`/`\vref` — resolved to numbers/names in a second pass, so **forward references work**
- `\input`/`\include` (parsed recursively with shared state), `\usepackage`, `\graphicspath`

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
- Inline: `$...$` and `\(...\)`; display: `$$...$$` and `\[...\]`
- Environments: `equation`, `align`, `gather`, `multline`, `flalign`, `alignat`, `eqnarray`, `split`, `displaymath` (+ starred forms). Multi-line environments are wrapped in `aligned`/`gathered` so KaTeX/MathJax render them
- `\label` in numbered environments becomes `\tag{n}`; `\eqref` renders `(n)`
- **User macros are expanded inside math** (`\newcommand{\R}{\mathbb{R}}` → `$\mathbb{R}$`)

### Tables
- `tabular`, `tabular*`, `tabularx`, `longtable`, `array`, `tabu` inside `table`/`table*`
- Column specs incl. `p{}/m{}/b{}/X`, `@{}`, `>{}`, `|`, `*{n}{spec}` repetition
- `\hline`, booktabs rules (`\toprule`/`\midrule`/`\bottomrule`/`\cmidrule(lr){2-3}`) filtered correctly
- `\multicolumn` (pads spanned cells), `\multirow`, `\\[len]` spacing
- Output is aligned and padded for raw-text readability

### Figures
- `figure`/`figure*`/`sidewaysfigure`/`wrapfigure`, `\includegraphics` (also inline), multiple images per figure, `\caption` with embedded `\label`

### Other Environments
- `quote`, `quotation`, `displayquote`, `verse`, `abstract`
- `verbatim` (captured raw — `%`, `$`, `\` inside code are safe), `lstlisting` (with `language=`), `minted`, `alltt`, `comment` (dropped)
- `center`, `flushleft`, `flushright`, `minipage`, `multicols`, `titlepage` (transparent)
- Theorem-like: built-in `theorem`/`lemma`/`proof`/... plus `\newtheorem{...}[shared]{...}[within]` with correct scoped numbering
- `tikzpicture` preserved as LaTeX code blocks; unknown environments preserved raw

### Macros
- `\newcommand`, `\renewcommand`, `\providecommand`, `\def`, `\DeclareRobustCommand` — braced and unbraced names, `[n]` parameter counts, optional-argument defaults, TeX-style `#1#2` parameter text
- Expansions are re-lexed and re-parsed, so macros producing formatting, math, or whole environments work
- `\newenvironment`/`\renewenvironment` — begin/end definitions expand at use sites (with arguments), composing correctly with built-in environments

### Links, Citations, Bibliography
- `\href{url}{text}` (with options), `\url`, `\hyperref[..]{...}`
- `\cite`, `\citep`, `\citet`, `\citeauthor`, `\citeyear`, `\parencite`, `\textcite`, ... with prenotes/postnotes
- **Numeric and author-year styles**: `\bibliographystyle{plainnat/apalike/...}` and `\usepackage[style=authoryear]{biblatex}` switch to natbib-style rendering — `Knuth (1984)`, `(Lamport, 1994; Knuth, 1984)`, `(see Goossens et al., 1993, p. 5)`
- Citations render in a post-parse pass, so numbers stay consistent with the References list even when the style/bibliography is declared at the end of the document
- `.bib` files via `\bibliography`/`\addbibresource` (robust character-level BibTeX parser: single-line entries, nested braces, quoted values, `@string`/`@comment`) → rendered References section
- `thebibliography`/`\bibitem` → numbered References section with resolved `[n]` markers

### Layout Commands
Print-only commands (`\vspace`, `\centering`, `\newpage`, `\setlength`, font size switches, ...) are consumed silently — including their arguments — instead of leaking into the output.

## Architecture

Texed follows a three-stage pipeline inspired by Pandoc:

1. **Lexer** (`src/lexer.rs`): Tokenizes LaTeX input into a stream of tokens
2. **Parser** (`src/parser.rs`): Builds an Abstract Syntax Tree (AST) from tokens
3. **Converter** (`src/converter.rs`): Transforms the AST into Markdown output

This architecture allows for:
- Clean separation of concerns
- Easy debugging with `--debug-tokens` and `--debug-ast`
- Extensibility for adding new LaTeX commands or output formats

## Development

### Running Tests

```bash
cargo test
```

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
- [ ] Unicode math symbol conversion
