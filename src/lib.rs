//! `texed` — a LaTeX to Markdown converter.
//!
//! The conversion pipeline is `Lexer -> Parser -> MarkdownConverter`; the
//! [`convert`] and [`convert_with`] functions run it end to end. Conversion
//! is total: any input string produces Markdown (malformed LaTeX degrades
//! gracefully instead of failing), and constructs that could not be
//! converted faithfully are reported as warnings.

pub mod citation;
pub mod cli;
pub mod commands;
pub mod converter;
pub mod decode;
pub mod error;
pub mod include_system;
pub mod lexer;
pub mod lexer_extended;
pub mod macro_processor;
pub mod parser;
pub mod state;

use std::path::PathBuf;

/// Options for [`convert_with`].
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Directory used to resolve `\input`, `\include`, `\bibliography`, ...
    pub base_path: Option<PathBuf>,
    /// Omit the YAML frontmatter block.
    pub no_frontmatter: bool,
}

/// Result of a conversion.
#[derive(Debug, Clone)]
pub struct Conversion {
    pub markdown: String,
    /// Constructs that could not be converted faithfully.
    pub warnings: Vec<String>,
}

/// Convert a LaTeX string to Markdown with default options.
pub fn convert(latex: &str) -> error::Result<String> {
    convert_with(latex, &Options::default()).map(|c| c.markdown)
}

/// Stack size of the conversion thread spawned by [`convert_with`].
const CONVERT_STACK_SIZE: usize = 256 * 1024 * 1024;

/// Convert a LaTeX string to Markdown.
///
/// The pipeline runs on a dedicated thread with a large stack, and any
/// internal panic is caught and reported as an error, so calling this is
/// safe on arbitrary (even adversarial) input.
pub fn convert_with(latex: &str, options: &Options) -> error::Result<Conversion> {
    let latex = latex.to_string();
    let options = options.clone();
    let handle = std::thread::Builder::new()
        .name("texed-convert".into())
        .stack_size(CONVERT_STACK_SIZE)
        .spawn(move || convert_on_current_thread(&latex, &options))
        .map_err(|e| error::TexedError::Internal(format!("cannot spawn conversion thread: {}", e)))?;
    handle.join().unwrap_or_else(|panic| {
        let detail = panic
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        Err(error::TexedError::Internal(detail))
    })
}

/// Run the pipeline on the calling thread (no stack or panic protection).
pub fn convert_on_current_thread(latex: &str, options: &Options) -> error::Result<Conversion> {
    let input = decode::normalize_text(latex);
    let (envs, commands) =
        include_system::prescan_verbatim_definitions(&input, options.base_path.as_deref());
    let tokens = lexer::Lexer::new(&input)
        .with_verbatim_envs(envs.iter())
        .with_verb_commands(commands.iter())
        .tokenize()?;
    let mut parser = parser::Parser::new(tokens).with_verbatim_definitions(&envs, &commands);
    if let Some(base) = &options.base_path {
        parser = parser.with_base_path(base.clone());
    }
    let document = parser.parse()?;
    let warnings = document.warnings.clone();
    let markdown = converter::MarkdownConverter::new()
        .with_frontmatter(!options.no_frontmatter)
        .convert(document)?;
    Ok(Conversion { markdown, warnings })
}
