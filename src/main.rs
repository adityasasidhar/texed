mod citation;
mod cli;
mod commands;
mod converter;
mod error;
mod include_system;
mod lexer;
mod lexer_extended;
mod macro_processor;
mod parser;
mod state;

use anyhow::Result;
use cli::Cli;

fn main() -> Result<()> {
    let cli = Cli::parse_args();
    cli.run()
}