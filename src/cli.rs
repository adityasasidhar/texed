use crate::converter::MarkdownConverter;
use crate::lexer::Lexer;
use crate::parser::{Block, Document, Inline, Parser};
use anyhow::{bail, Context};
use clap::{CommandFactory, Parser as ClapParser};
use clap_complete::Shell;
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

const AFTER_HELP: &str = "\
\x1b[1;4mExamples:\x1b[0m
  \x1b[1mtexed paper.tex\x1b[0m                     Convert to stdout
  \x1b[1mtexed paper.tex -o paper.md\x1b[0m         Convert to a file
  \x1b[1mtexed ch1.tex ch2.tex ch3.tex\x1b[0m       Batch: writes ch1.md, ch2.md, ch3.md
  \x1b[1mtexed src/*.tex -o build/\x1b[0m           Batch into a directory
  \x1b[1mcat notes.tex | texed\x1b[0m               Read from stdin
  \x1b[1mtexed paper.tex --check\x1b[0m             Report conversion fidelity issues
  \x1b[1mtexed paper.tex -o out.md --stats\x1b[0m   Show timing and document statistics
  \x1b[1mtexed --completions zsh\x1b[0m             Generate shell completions

\x1b[1;4mExit codes:\x1b[0m
  0  success
  1  conversion failed, or warnings found with --check/--strict
";

#[derive(ClapParser, Debug)]
#[command(
    name = "texed",
    version,
    author,
    about = "Convert LaTeX documents to Markdown",
    long_about = "    (\\ _/)\n    ( \u{2022}_\u{2022})\n   / >\u{1F980}  Texed\n\n\\section{Hello}  \u{2192}  ## Hello\n\n\
                  A command-line tool to convert LaTeX documents to Markdown format.\n\
                  Handles sectioning, math, tables, figures, macros, citations,\n\
                  cross-references, and 200+ common LaTeX commands.",
    after_help = AFTER_HELP,
    arg_required_else_help = false
)]
pub struct Cli {
    /// Input LaTeX file(s); use '-' or omit to read from stdin
    #[arg(value_name = "INPUT")]
    pub inputs: Vec<PathBuf>,

    /// Output file, or directory when converting multiple files ('-' for stdout)
    #[arg(short, long, value_name = "PATH")]
    pub output: Option<PathBuf>,

    /// Overwrite output files if they exist
    #[arg(short = 'f', long)]
    pub force: bool,

    /// Show progress, timings, and every conversion warning
    #[arg(short, long, conflicts_with = "quiet")]
    pub verbose: bool,

    /// Suppress all diagnostics except errors
    #[arg(short, long)]
    pub quiet: bool,

    /// Omit the YAML frontmatter block (title/author/date)
    #[arg(long)]
    pub no_frontmatter: bool,

    /// Analyze conversion fidelity without writing output;
    /// exits 1 if any construct could not be converted faithfully
    #[arg(long)]
    pub check: bool,

    /// Treat conversion warnings as errors
    #[arg(long)]
    pub strict: bool,

    /// Print document statistics after converting
    #[arg(long)]
    pub stats: bool,

    /// Generate shell completions and exit
    #[arg(long, value_enum, value_name = "SHELL", exclusive = true)]
    pub completions: Option<Shell>,

    /// Pretty print the parsed AST (for debugging)
    #[arg(long, help_heading = "Debug")]
    pub debug_ast: bool,

    /// Show tokens from lexer (for debugging)
    #[arg(long, help_heading = "Debug")]
    pub debug_tokens: bool,
}

/// One conversion unit: where the LaTeX comes from and where the Markdown goes.
struct Job {
    input: InputSource,
    output: OutputTarget,
}

enum InputSource {
    Stdin,
    File(PathBuf),
}

enum OutputTarget {
    Stdout,
    File(PathBuf),
}

impl InputSource {
    fn display(&self) -> String {
        match self {
            InputSource::Stdin => "<stdin>".to_string(),
            InputSource::File(path) => path.display().to_string(),
        }
    }
}

impl Cli {
    pub fn parse_args() -> Self {
        <Self as ClapParser>::parse()
    }

    pub fn run(&self) -> anyhow::Result<()> {
        if let Some(shell) = self.completions {
            let mut cmd = Self::command();
            clap_complete::generate(shell, &mut cmd, "texed", &mut io::stdout());
            return Ok(());
        }

        let jobs = self.collect_jobs()?;
        let multi = jobs.len() > 1;

        let mut total_warnings = 0usize;
        let mut failures = 0usize;
        for job in &jobs {
            match self.convert_one(job, multi) {
                Ok(warnings) => total_warnings += warnings,
                Err(error) => {
                    failures += 1;
                    eprintln!(
                        "{} {}: {:#}",
                        paint("error:", RED_BOLD),
                        job.input.display(),
                        error
                    );
                }
            }
        }

        if failures > 0 {
            bail!("{} of {} file(s) failed to convert", failures, jobs.len());
        }
        if self.strict && total_warnings > 0 {
            bail!(
                "--strict: {} conversion warning(s) treated as errors",
                total_warnings
            );
        }
        if self.check {
            if total_warnings > 0 {
                bail!("--check: {} conversion warning(s) found", total_warnings);
            }
            if !self.quiet {
                eprintln!(
                    "{} all constructs converted faithfully",
                    paint("check passed:", GREEN_BOLD)
                );
            }
        }

        Ok(())
    }

    /// Pair every input with its output target.
    fn collect_jobs(&self) -> anyhow::Result<Vec<Job>> {
        let is_stdin = |p: &PathBuf| p.to_str() == Some("-");

        // No inputs (or a single '-') means stdin
        if self.inputs.is_empty() || (self.inputs.len() == 1 && is_stdin(&self.inputs[0])) {
            let output = match &self.output {
                Some(path) if path.to_str() != Some("-") => {
                    OutputTarget::File(path.clone())
                }
                _ => OutputTarget::Stdout,
            };
            return Ok(vec![Job {
                input: InputSource::Stdin,
                output,
            }]);
        }

        if self.inputs.len() > 1 && self.inputs.iter().any(is_stdin) {
            bail!("'-' (stdin) cannot be combined with other input files");
        }

        let multi = self.inputs.len() > 1;
        let output_is_stdout = self.output.as_deref().and_then(Path::to_str) == Some("-");
        let output_is_dir = match &self.output {
            Some(path) if !output_is_stdout => {
                multi || path.is_dir() || path.as_os_str().to_string_lossy().ends_with('/')
            }
            _ => false,
        };

        if output_is_dir {
            let dir = self.output.as_ref().unwrap();
            fs::create_dir_all(dir).with_context(|| {
                format!("Failed to create output directory: {}", dir.display())
            })?;
        }

        let mut jobs = Vec::new();
        for input in &self.inputs {
            let output = if output_is_stdout {
                OutputTarget::Stdout
            } else if output_is_dir {
                let dir = self.output.as_ref().unwrap();
                OutputTarget::File(dir.join(markdown_name(input)))
            } else if let Some(path) = &self.output {
                OutputTarget::File(path.clone())
            } else if multi {
                // Batch default: sibling .md next to each input
                OutputTarget::File(input.with_extension("md"))
            } else {
                OutputTarget::Stdout
            };
            jobs.push(Job {
                input: InputSource::File(input.clone()),
                output,
            });
        }
        Ok(jobs)
    }

    /// Convert one file; returns the number of conversion warnings.
    fn convert_one(&self, job: &Job, multi: bool) -> anyhow::Result<usize> {
        let start = Instant::now();
        let input_content = self.read_input(&job.input)?;
        if self.verbose {
            eprintln!("Read {} bytes from {}", input_content.len(), job.input.display());
        }

        let lex_start = Instant::now();
        let mut lexer = Lexer::new(&input_content);
        let tokens = lexer.tokenize().context("Failed to tokenize LaTeX input")?;
        let lex_time = lex_start.elapsed();

        if self.debug_tokens {
            eprintln!("=== TOKENS ===");
            for (i, token) in tokens.iter().enumerate() {
                eprintln!("{:4}: {:?}", i, token);
            }
            eprintln!("=== END TOKENS ===\n");
        }
        if self.verbose {
            eprintln!("Tokenized into {} tokens", tokens.len());
        }

        let parse_start = Instant::now();
        let mut parser = Parser::new(tokens);
        if let InputSource::File(path) = &job.input {
            if let Some(base) = path.parent() {
                parser = parser.with_base_path(base.to_path_buf());
            }
        }
        let document = parser.parse().context("Failed to parse LaTeX document")?;
        let parse_time = parse_start.elapsed();

        if self.debug_ast {
            eprintln!("=== AST ===");
            eprintln!("{:#?}", document);
            eprintln!("=== END AST ===\n");
        }
        if self.verbose {
            eprintln!("Parsed into {} blocks", document.blocks.len());
        }

        let warnings = document.warnings.clone();
        let stats = if self.stats {
            Some(DocumentStats::collect(&document))
        } else {
            None
        };

        let convert_start = Instant::now();
        let mut converter = MarkdownConverter::new().with_frontmatter(!self.no_frontmatter);
        let markdown = converter
            .convert(document)
            .context("Failed to convert to Markdown")?;
        let convert_time = convert_start.elapsed();

        if self.verbose {
            eprintln!("Generated {} bytes of Markdown", markdown.len());
        }

        if !self.check {
            self.write_output(&job.output, &markdown)?;
        }

        // Per-file progress line for batch conversions
        if (multi || self.check) && !self.quiet {
            let target = match (&job.output, self.check) {
                (_, true) => "checked".to_string(),
                (OutputTarget::Stdout, _) => "stdout".to_string(),
                (OutputTarget::File(path), _) => path.display().to_string(),
            };
            let note = if warnings.is_empty() {
                String::new()
            } else {
                format!(
                    " ({} warning{})",
                    warnings.len(),
                    if warnings.len() == 1 { "" } else { "s" }
                )
            };
            eprintln!(
                "{} {} \u{2192} {}{}",
                paint("\u{2713}", GREEN_BOLD),
                job.input.display(),
                target,
                paint(&note, YELLOW)
            );
        }

        self.report_warnings(&warnings);

        if let Some(stats) = stats {
            stats.print(
                &job.input.display(),
                input_content.len(),
                markdown.len(),
                lex_time.as_secs_f64() * 1000.0,
                parse_time.as_secs_f64() * 1000.0,
                convert_time.as_secs_f64() * 1000.0,
            );
        }

        if self.verbose {
            eprintln!(
                "Conversion completed successfully in {:.1} ms",
                start.elapsed().as_secs_f64() * 1000.0
            );
        }

        Ok(warnings.len())
    }

    /// Print conversion warnings: full deduplicated list in --check/--verbose,
    /// a one-line summary otherwise.
    fn report_warnings(&self, warnings: &[String]) {
        if warnings.is_empty() || self.quiet {
            return;
        }

        if self.check || self.verbose {
            // Deduplicate, preserving first-seen order
            let mut counts: Vec<(&String, usize)> = Vec::new();
            for warning in warnings {
                match counts.iter_mut().find(|(msg, _)| *msg == warning) {
                    Some((_, n)) => *n += 1,
                    None => counts.push((warning, 1)),
                }
            }
            for (message, count) in counts {
                if count > 1 {
                    eprintln!(
                        "{} {} {}",
                        paint("warning:", YELLOW_BOLD),
                        message,
                        paint(&format!("(\u{00D7}{})", count), DIM)
                    );
                } else {
                    eprintln!("{} {}", paint("warning:", YELLOW_BOLD), message);
                }
            }
        } else {
            eprintln!(
                "{} {} conversion note{} \u{2014} run with --check for details",
                paint("warning:", YELLOW_BOLD),
                warnings.len(),
                if warnings.len() == 1 { "" } else { "s" }
            );
        }
    }

    fn read_input(&self, input: &InputSource) -> anyhow::Result<String> {
        match input {
            InputSource::Stdin => {
                if self.verbose {
                    eprintln!("Reading from stdin...");
                }
                let mut buffer = String::new();
                io::stdin()
                    .read_to_string(&mut buffer)
                    .context("Failed to read from stdin")?;
                Ok(buffer)
            }
            InputSource::File(path) => fs::read_to_string(path)
                .with_context(|| format!("Failed to read input file: {}", path.display())),
        }
    }

    fn write_output(&self, output: &OutputTarget, content: &str) -> anyhow::Result<()> {
        match output {
            OutputTarget::Stdout => {
                io::stdout()
                    .write_all(content.as_bytes())
                    .context("Failed to write to stdout")?;
            }
            OutputTarget::File(path) => {
                if path.exists() && !self.force {
                    bail!(
                        "Output file already exists: {}. Use --force to overwrite.",
                        path.display()
                    );
                }
                fs::write(path, content)
                    .with_context(|| format!("Failed to write output file: {}", path.display()))?;
            }
        }
        Ok(())
    }
}

/// Derive the output filename for an input: same stem, .md extension.
fn markdown_name(input: &Path) -> PathBuf {
    let stem = input.file_stem().unwrap_or(input.as_os_str());
    let mut name = PathBuf::from(stem);
    name.set_extension("md");
    name
}

// ---------------------------------------------------------------------------
// Terminal colors (stderr only, disabled when piped or NO_COLOR is set)

const RED_BOLD: &str = "\x1b[1;31m";
const GREEN_BOLD: &str = "\x1b[1;32m";
const YELLOW: &str = "\x1b[33m";
const YELLOW_BOLD: &str = "\x1b[1;33m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

fn color_enabled() -> bool {
    io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

fn paint(text: &str, code: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    if color_enabled() {
        format!("{}{}{}", code, text, RESET)
    } else {
        text.to_string()
    }
}

// ---------------------------------------------------------------------------
// Document statistics (--stats)

#[derive(Default)]
struct DocumentStats {
    headings: usize,
    paragraphs: usize,
    lists: usize,
    tables: usize,
    figures: usize,
    equations: usize,
    code_blocks: usize,
    quotes: usize,
    theorems: usize,
    inline_math: usize,
    citations: usize,
    links: usize,
    footnotes: usize,
    /// Table cells hold paragraphs internally; don't count those
    in_table: bool,
}

impl DocumentStats {
    fn collect(document: &Document) -> Self {
        let mut stats = Self {
            footnotes: document.footnotes.len(),
            ..Self::default()
        };
        stats.walk_blocks(&document.blocks);
        stats
    }

    fn walk_blocks(&mut self, blocks: &[Block]) {
        for block in blocks {
            match block {
                Block::Section { title, .. } => {
                    self.headings += 1;
                    self.walk_inlines(title);
                }
                Block::Paragraph(inlines) => {
                    if !self.in_table {
                        self.paragraphs += 1;
                    }
                    self.walk_inlines(inlines);
                }
                Block::BulletList(items) | Block::OrderedList { items, .. } => {
                    self.lists += 1;
                    for item in items {
                        self.walk_blocks(item);
                    }
                }
                Block::DescriptionList(items) => {
                    self.lists += 1;
                    for (term, description) in items {
                        self.walk_inlines(term);
                        self.walk_blocks(description);
                    }
                }
                Block::Quote(blocks) => {
                    self.quotes += 1;
                    self.walk_blocks(blocks);
                }
                Block::Composite(blocks) => self.walk_blocks(blocks),
                Block::CodeBlock { .. } | Block::Verbatim(_) => self.code_blocks += 1,
                Block::DisplayMath(_) => self.equations += 1,
                Block::Table {
                    caption,
                    headers,
                    rows,
                    ..
                } => {
                    self.tables += 1;
                    if let Some(caption) = caption {
                        self.walk_inlines(caption);
                    }
                    let was_in_table = self.in_table;
                    self.in_table = true;
                    for cell in headers {
                        self.walk_blocks(cell);
                    }
                    for row in rows {
                        for cell in row {
                            self.walk_blocks(cell);
                        }
                    }
                    self.in_table = was_in_table;
                }
                Block::Figure { caption, .. } => {
                    self.figures += 1;
                    if let Some(caption) = caption {
                        self.walk_inlines(caption);
                    }
                }
                Block::TheoremLike { content, .. } => {
                    self.theorems += 1;
                    self.walk_blocks(content);
                }
                Block::RawBlock(_) | Block::HorizontalRule | Block::Null => {}
            }
        }
    }

    fn walk_inlines(&mut self, inlines: &[Inline]) {
        for inline in inlines {
            match inline {
                Inline::InlineMath(_) => self.inline_math += 1,
                Inline::Cite { .. } => self.citations += 1,
                Inline::Link { text, .. } => {
                    self.links += 1;
                    self.walk_inlines(text);
                }
                Inline::Emph(content)
                | Inline::Strong(content)
                | Inline::Strikeout(content)
                | Inline::Underline(content)
                | Inline::Superscript(content)
                | Inline::Subscript(content)
                | Inline::SmallCaps(content)
                | Inline::Span { content, .. }
                | Inline::Quoted { content, .. }
                | Inline::Image { alt: content, .. } => self.walk_inlines(content),
                Inline::Note(blocks) => self.walk_blocks(blocks),
                _ => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn print(
        &self,
        input_name: &str,
        input_bytes: usize,
        output_bytes: usize,
        lex_ms: f64,
        parse_ms: f64,
        convert_ms: f64,
    ) {
        let total_ms = lex_ms + parse_ms + convert_ms;
        eprintln!();
        eprintln!("{}", paint(&format!("  {}", input_name), BOLD));
        eprintln!(
            "  blocks   {} headings \u{00B7} {} paragraphs \u{00B7} {} lists \u{00B7} {} tables \u{00B7} {} figures \u{00B7} {} equations \u{00B7} {} code \u{00B7} {} theorems",
            self.headings,
            self.paragraphs,
            self.lists,
            self.tables,
            self.figures,
            self.equations,
            self.code_blocks,
            self.theorems,
        );
        eprintln!(
            "  inline   {} math \u{00B7} {} citations \u{00B7} {} links \u{00B7} {} footnotes",
            self.inline_math, self.citations, self.links, self.footnotes,
        );
        eprintln!(
            "  size     {} \u{2192} {}",
            human_bytes(input_bytes),
            human_bytes(output_bytes),
        );
        eprintln!(
            "  time     {:.1} ms {}",
            total_ms,
            paint(
                &format!(
                    "(lex {:.1} \u{00B7} parse {:.1} \u{00B7} convert {:.1})",
                    lex_ms, parse_ms, convert_ms
                ),
                DIM
            ),
        );
        eprintln!();
    }
}

fn human_bytes(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_markdown_name() {
        assert_eq!(markdown_name(Path::new("a/b/paper.tex")), PathBuf::from("paper.md"));
        assert_eq!(markdown_name(Path::new("notes")), PathBuf::from("notes.md"));
    }

    #[test]
    fn test_human_bytes() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn test_cli_command_builds() {
        // Catches invalid clap attribute combinations at test time
        Cli::command().debug_assert();
    }
}
