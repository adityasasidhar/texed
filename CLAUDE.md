# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`texed` is a Rust CLI that converts LaTeX documents to Markdown (see `README.md` for user-facing docs and supported LaTeX commands). Single binary crate, no workspace, no async runtime.

## Commands

```bash
cargo build                              # debug build
cargo build --release                    # release build -> target/release/texed
cargo run -- input.tex -o output.md      # run against a file
cargo run -- input.tex --debug-tokens --debug-ast -v   # inspect lexer/parser output while developing
cargo run -- input.tex --check           # list every construct that didn't convert faithfully
                                         # (unknown commands/envs, unresolved refs) — the fastest
                                         # dev loop when adding support for a new LaTeX command

cargo test                               # unit tests (in-module, src/**) + tests/integration_test.rs
cargo test <test_name>                   # run a single test by name substring, e.g. `cargo test test_convert_section`
cargo test --test integration_test       # only the CLI-level integration tests
cargo test --lib <name>                  # only the in-module unit tests

cargo clippy                             # lint (no rustfmt.toml / clippy.toml in the repo; defaults apply)
```

There is no CI config in the repo (`.github/` does not exist) — `cargo build`/`cargo test`/`cargo clippy` are run manually.

All tests pass on a clean checkout (`cargo test`). Heading convention: H1 is reserved for the document title (`\maketitle`/`\chapter`/`\part`), so `\section` maps to `##` — tests assert this; don't change the mapping without updating them.

## Architecture

Three-stage pipeline (`src/main.rs` wires it end to end via `cli.rs`):

```
LaTeX text --Lexer--> Vec<Token> --Parser--> Document (AST of Block/Inline) --MarkdownConverter--> Markdown string
```

- **`lexer.rs`** — `Lexer::tokenize()` turns raw LaTeX into a flat `Vec<Token>` (commands, `\begin`/`\end`, braces, math delimiters, sectioning, etc.). Whitespace is preserved as tokens rather than skipped, since the parser decides its significance.
- **`parser.rs`** — the largest file (~1800 lines) and the core of the project. `Parser` consumes tokens and recursively builds a `Document { metadata, blocks, footnotes, bibliography }` out of `Block`/`Inline` enums (defined at the top of this file — check them first when adding support for a new LaTeX construct). Key entry points: `parse_block`, `parse_environment` (handles `\begin{...}...\end{...}`), `parse_command_block`.
- **`converter.rs`** — `MarkdownConverter::convert()` walks the `Document`/`Block`/`Inline` tree and emits a Markdown `String`. This is where output formatting decisions live (e.g. table alignment syntax, heading depth, footnote placement at the end of the document).
- **`state.rs`** — `ParserState`: mutable cross-cutting parse state threaded through `Parser` (label/reference table for `\ref`/`\label`, footnote/figure/table/equation/theorem counters, macro table, theorem-environment definitions, toggles, bibliography list). Anything that needs to persist or be looked up across the whole document (not just within one block) lives here.
- **`macro_processor.rs`** — `MacroProcessor` expands user `\newcommand`/`\renewcommand`/`\providecommand` macros plus a set of built-in zero-arg macros (`\LaTeX`, `\ie`, `\smallskip`, etc.). Consulted by the parser via `is_defined`/`expand_macro`/`get_definition`.
- **`include_system.rs`** — `IncludeSystem` resolves `\input`/`\include`/`\usepackage` and `\includegraphics` file paths (TEXINPUTS-style search paths, include-depth cycle protection). The parser recursively re-invokes itself on included file contents (`parse_included_content` in `parser.rs`) by lexing the included text and running a **child `Parser`** that shares/merges `state`, `citation_manager`, `macro_processor`, `include_system`, and `command_registry` with the parent so cross-references and macros stay consistent across files.
- **`citation.rs`** — `CitationManager`/`BibEntry` parse `.bib` files and render `\cite`-style citations and the bibliography block; invoked from `parser.rs` after all blocks are parsed (`load_bibliography_files` / `render_bibliography`).
- **`commands.rs`** — `CommandRegistry`: a `HashMap<String, fn(&str, &[String]) -> Result<String>>` of handlers for LaTeX inline text-formatting commands (`\textbf`, `\href`, `\underline`, etc.) that don't need their own `Inline` variant — they're expanded to a Markdown/HTML string directly. Register new simple one-arg commands here rather than adding parser special-casing.
- **`lexer_extended.rs`** — `ExtendedToken`, `AccentType`, `SpecialCharType`, and `get_accent_type`/`get_special_char` lookup tables, paired with `apply_accent`/`special_char_to_string` in `commands.rs`. As of this writing these are **not wired into `lexer.rs` or `parser.rs`** — accent commands (`\'{e}`, `\~{n}`, etc.) and special-character macros aren't yet dispatched through this module from the main pipeline. Check call sites before assuming accent handling works end-to-end.
- **`error.rs`** — `TexedError` (via `thiserror`) is the crate-internal error type used by `lexer`/`parser`/`converter` (`error::Result<T>`). The CLI layer (`cli.rs`, `main.rs`) instead uses `anyhow::Result` with `.context(...)` for user-facing error messages.

### Adding support for a new LaTeX command/environment

1. If it's a simple one-argument inline command that maps to a Markdown/HTML snippet, add a handler in `commands.rs::register_default_commands` — no parser changes needed.
2. If it needs structure (new AST shape, arguments with specific parsing rules, or block-level behavior), add a variant to `Block`/`Inline` in `parser.rs`, handle it in `parse_block`/`parse_environment`/`parse_command_block`, and add a corresponding `convert_*` case in `converter.rs`.
3. If it needs to be tracked/numbered/cross-referenced across the document, add the counter/table to `ParserState` in `state.rs`.

### Testing conventions

- Unit tests live in `#[cfg(test)] mod tests` blocks at the bottom of each `src/*.rs` file and typically test one module in isolation (e.g. `lexer.rs` tests only tokenization).
- `tests/integration_test.rs` drives the compiled binary end-to-end via `assert_cmd::Command::cargo_bin("texed")`, writing a `.tex` fixture to a `tempfile::TempDir`, running the CLI, and asserting on the output file contents.
