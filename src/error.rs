use thiserror::Error;

#[derive(Error, Debug)]
pub enum TexedError {
    #[error("Failed to read input file: {0}")]
    InputFileError(#[from] std::io::Error),

    #[error("LaTeX parsing error at line {line}, column {col}: {message}")]
    ParseError {
        line: usize,
        col: usize,
        message: String,
    },

    #[error("Unsupported LaTeX command: {0}")]
    UnsupportedCommand(String),

    #[error("Unbalanced braces: expected closing brace")]
    UnbalancedBraces,

    #[error("Invalid LaTeX syntax: {0}")]
    InvalidSyntax(String),

    #[error("Failed to write output: {0}")]
    OutputError(String),
}

pub type Result<T> = std::result::Result<T, TexedError>;
