# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`texed` is a Rust LaTeX → Markdown converter (see `README.md` for user-facing docs). One crate with a library target (`src/lib.rs`, public `texed::convert` / `texed::convert_with`) and a thin CLI binary (`src/main.rs` → `cli.rs`). No workspace, no async runtime.

Design contract: **conversion is total**. Any input — malformed, adversarial, non-UTF-8, enormous — produces Markdown without panicking, hanging, or blowing up memory; constructs that can't be converted faithfully become *warnings* (`--check`), never errors. Keep that property when changing anything.

## Commands

```bash
cargo build --release                    # release build -> target/release/texed
cargo run -- input.tex -o output.md      # run against a file
cargo run -- input.tex --debug-tokens --debug-ast -v   # inspect lexer/parser output
cargo run -- input.tex --check           # list every construct that didn't convert faithfully
                                         # (unknown commands/envs, unresolved refs) — the fastest
                                         # dev loop when adding support for a new LaTeX command

cargo test --release                     # everything (release: the fuzz tests are slow in debug)
cargo test --release --test features     # one focused test per supported construct
cargo test --release --test markdown_safety  # escaping verified against a CommonMark parser
cargo test --release --test robustness   # fuzzing: never panic/hang/explode
cargo test --release --test integration_test # CLI-level tests (assert_cmd)
cargo test --lib <name>                  # in-module unit tests

cargo clippy --release --all-targets     # must stay warning-free
```

There is no CI config in the repo — run `cargo test --release` and `cargo clippy --all-targets` manually. The robustness tests can use a few GB of RAM briefly; use `-- --test-threads=2` on small machines.

Heading convention: H1 is reserved for the document title (`\maketitle`/`\chapter`/`\part`), so `\section` maps to `##` — tests assert this.

## Architecture

```
bytes --decode--> text --Lexer--> Vec<Token> --Parser--> Document (Block/Inline AST) --MarkdownConverter--> Markdown
```

- **`decode.rs`** — bytes → normalized text: UTF-8 (BOM stripped), UTF-16 (BOM or heuristic), Windows-1252 fallback; CRLF/CR → LF. All file reads (main file, `\input`, `.sty`, `.bib`) go through `decode::read_file`.
- **`lexer.rs`** — `Lexer::tokenize()` never fails. Notable behavior: math (`$..$`, `\(..\)`, `\[..\]`, and math environments → `Token::MathEnv`) and verbatim-like/graphics environments (verbatim, lstlisting, minted, tikzpicture, ...) are captured **raw**; URL arguments are verbatim; unterminated constructs are bounded by TeX's own rules (no paragraph break in `$`, no newline in `\verb`, simple groups for `\begin{`/`\label{`). `\makeatletter`/`\ExplSyntaxOn` change what counts as a letter. `with_math_mode(true)` lexes math so tokens serialize back exactly (used for macro expansion in math).
- **`parser/mod.rs`** — core `Parser`. The token stream is a **reversed stack** (`tokens.last()` is current): `advance` pops, `push_front` splices an expansion in O(k). Never index tokens by position; use `current_token`/`peek_at`/`skip_n`/`push_front`. Safety invariants: `MAX_NESTING_DEPTH` bounds recursion (past it tokens are flattened), `parse_block`/`parse_inline` always consume at least one token (progress guarantee via the `consumed` counter), macro expansion is budgeted (count, per-macro, bytes, nesting — see `consume_expansion_budget`/`consume_expansion_bytes`). Material before `\begin{document}` is parsed only for side effects (`in_preamble`); parsing stops at `\end{document}`.
  Spliced files end with marker commands (`texed@endfile`, `texed@popdir`, `texed@endpackage`; see `is_file_end_marker`): skipped conditionals, unclosed groups and raw captures stop there. `\documentclass[main]{subfiles}` splices the main file's preamble (`load_subfiles_main`). A `\label` or `\caption` in running text attaches to the innermost `LabelTarget` (generic floats, theorems), else to the current section.
  Dispatch order in `parse_inline_command`: `\let` aliases → TeX-level commands (`tex.rs`) → definitions mid-paragraph → `\input` splicing → refs/cites/footnotes → user macros → `parse_builtin_command` (counters, accents, symbols, no-op layout commands, `parse_knowledge_command`, formatting commands) → `parse_unknown_command` (drops the name, keeps argument text, warns).
- **`parser/tex.rs`** — TeX programming: `\if..\else..\fi` (incl. `\newif`, `\ifx`, `\ifnum`, `\ifcase`, TeX-faithful `\if` operands), etoolbox/ifthen braced conditionals and list loops (`\docsvlist`, `\listadd`, kernel `\@for`), kernel helpers (`\@ifnextchar`, `\@ifstar`, `\@firstoftwo`, ...), `\let` (captures built-in meaning via `BUILTIN_MARK`), `\csname`, `\expandafter` (`expand_after`/`expand_once`: one-level expansion of the following token), counter formatting, the recursion-aware expansion budget, and `expand_math_with_engine` (macro expansion inside math using this same engine). `lex_fragment` re-lexes macro bodies with TeX's rule that spaces/one line end after a control word are dropped (document text keeps them).
- **expl3** (`parse_expl3_definition` in mod.rs): `\cs_new:Npn`/`:Nn` define macros, `\cs_set_eq:NN` is `\let`, token-list variables are parameterless macros, `\use:x` is x-expanded (`x_expand_tokens`), other expl3 functions are dropped by signature. Under `\ExplSyntaxOn`/`\ProvidesExplPackage` the lexer ignores spaces and maps primitive names (`\if_meaning:w`, `\fi:` ...) to TeX names (`expl_primitive`).
- **`parser/knowledge.rs`** — data tables: set-up commands with argument signatures (consumed silently), TeX register assignments (`\parskip 0pt plus 1pt`), text-mode math symbols, ltxdoc markup, implicit theorem names. **Most "support command X" work is adding a table row here.**
- **`parser/{tables,figures,algorithms,glossary,siunitx,math}.rs`** — tables/longtable/tabular bodies; figure floats and subfigures; pseudocode (algorithmicx, algorithmic, algorithm2e); acronyms/glossaries; siunitx; math clean-up.
- **`converter.rs`** — `MarkdownConverter` walks the AST. Output must be *Markdown-safe*: `escape_markdown_text` (text), placeholder brackets/ampersands resolved by `finalize_brackets` per paragraph, `escape_line_starts` (`#`, `>`, `- `, `1. `...), `convert_delimited` (emphasis with HTML fallback when CommonMark flanking would fail), fence lengths from `longest_run`, `markdown_url`, `yaml_scalar`. `tests/markdown_safety.rs` checks all of this with pulldown-cmark — extend it when adding output syntax.
- **`state.rs`** — `ParserState`: labels, counters and numbering (`next_counter`, `\appendix`, per-chapter numbering, `\numberwithin`), macros (`MacroDef` incl. TeX delimited parameters), theorem envs, conditionals, aliases, warnings. `substitute_params` does single-pass `#n` substitution.
- **`macro_processor.rs`** — built-in zero-arg macros (logos, abbreviations) + lookups over `state.macros`.
- **`include_system.rs`** — resolves `\input`/`\include`/`\usepackage` (local `.sty`/`.cls` files are read and spliced) and graphics paths; include-depth/cycle protection.
- **`citation.rs`**, **`commands.rs`**, **`lexer_extended.rs`** — `.bib` parsing and citation rendering (fields stay LaTeX; `format_citation`/`bibliography_items` return LaTeX that the parser renders via `latex_to_inlines`); simple one-arg command handlers; accent/special-character tables (used by `parse_builtin_command`).
- **`error.rs`** — `TexedError` (library); the CLI uses `anyhow`. `TexedError::Internal` reports a caught panic.

### Adding support for a LaTeX command/environment

1. Pure set-up (no output)? Add a `Signature` row to `knowledge::setup_command`.
2. Simple text/symbol? `text_symbol_command` (mod.rs), `knowledge::text_math_symbol`, or a built-in macro in `macro_processor.rs`.
3. Structure? A branch in `parse_environment` / `parse_builtin_command` (or the relevant module), possibly a new `Block`/`Inline` variant with a `convert_*` case.
4. Numbered / cross-referenced? Counters and labels live in `ParserState`.
5. Add a case to `tests/features.rs`; run `--check` on a real document using it.

### Testing conventions

- `#[cfg(test)] mod tests` blocks test modules in isolation; `tests/features.rs` has one focused case per construct via `texed::convert_with`.
- `tests/integration_test.rs` drives the binary (`assert_cmd`), writing fixtures to a `tempfile::TempDir`.
- `tests/robustness.rs` fuzzes with a deterministic PRNG (fragment soup, random bytes, every prefix of `tests/fixtures/kitchen_sink.tex`, deep nesting, recursive macros). Any new parsing loop must keep these passing.
- `tests/markdown_safety.rs` round-trips random special-character text through LaTeX → Markdown → pulldown-cmark in many contexts.
