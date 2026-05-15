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

Convert a LaTeX file to Markdown:

```bash
texed input.tex -o output.md
```

### Read from stdin, write to stdout:

```bash
cat input.tex | texed > output.md
```

### Using stdin explicitly:

```bash
texed - -o output.md < input.tex
```

### Command-Line Options

```
Usage: texed [OPTIONS] [INPUT]

Arguments:
  [INPUT]  Input LaTeX file (use '-' for stdin)

Options:
  -o, --output <OUTPUT>  Output Markdown file (use '-' for stdout, default: stdout)
  -f, --force            Overwrite output file if it exists
  -v, --verbose          Verbose output
      --debug-ast        Pretty print the parsed AST (for debugging)
      --debug-tokens     Show tokens from lexer (for debugging)
  -h, --help             Print help
  -V, --version          Print version
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

| Name | Age | City |
| :--- | :---: | :---: |
| Alice | 30 | NYC |
| Bob | 25 | LA |
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
- `\section`, `\subsection`, `\subsubsection`
- `\chapter`, `\paragraph`, `\subparagraph`
- `\label`, `\ref`

### Text Formatting
- `\textbf`, `\textit`, `\texttt`
- `\emph`, `\underline`
- `\textsuperscript`, `\textsubscript`
- `\textsc` (small caps)

### Lists
- `\begin{itemize}...\end{itemize}`
- `\begin{enumerate}...\end{enumerate}`
- `\begin{description}...\end{description}`
- `\item`

### Math
- Inline: `$...$`
- Display: `$$...$$` or `\[...\]`
- Environments: `equation`, `align`, `gather`

### Tables
- `\begin{table}...\end{table}`
- `\begin{tabular}...\end{tabular}`
- `\caption`

### Figures
- `\begin{figure}...\end{figure}`
- `\includegraphics`
- `\caption`

### Other Environments
- `\begin{quote}...\end{quote}`
- `\begin{verbatim}...\end{verbatim}`
- `\begin{lstlisting}...\end{lstlisting}`
- Theorem-like: `theorem`, `lemma`, `proof`, etc.

### Links and References
- `\href{url}{text}`
- `\url{url}`
- `\cite`, `\citep`, `\citet`

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

- [ ] Support for more LaTeX packages (amsmath, tikz, etc.)
- [ ] Bibliography and citation handling
- [ ] Custom command definitions (`\newcommand`)
- [ ] Multiple output formats (HTML, reStructuredText)
- [ ] Configuration file support
- [ ] Better error messages with line/column information
- [ ] Unicode math symbol conversion
