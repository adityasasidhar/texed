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

use cli::Cli;
use std::io::IsTerminal;

fn main() {
    let cli = Cli::parse_args();
    if let Err(error) = cli.run() {
        let red = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        let prefix = if red { "\x1b[1;31merror:\x1b[0m" } else { "error:" };
        eprintln!("{} {:#}", prefix, error);
        std::process::exit(1);
    }
}
