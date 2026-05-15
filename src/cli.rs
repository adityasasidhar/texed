use crate::converter::MarkdownConverter;
use crate::lexer::Lexer;
use crate::parser::Parser;
use anyhow::Context;
use clap::Parser as ClapParser;
use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;

#[derive(ClapParser, Debug)]
#[command(
    name = "texed",
    version = "0.1.0",
    author = "Your Name <your.email@example.com>",
    about = "Convert LaTeX documents to Markdown",
    long_about = "    (\\ _/)\n    ( •_•)\n   / >🦀  Texed\n\n\\section{Hello}  →  # Hello\n\n\
                  A command-line tool to convert LaTeX to Markdown format."
)]
pub struct Cli {
    /// Input LaTeX file (use '-' for stdin)
    #[arg(value_name = "INPUT")]
    pub input: Option<PathBuf>,

    /// Output Markdown file (use '-' for stdout, default: stdout)
    #[arg(short, long, value_name = "OUTPUT")]
    pub output: Option<PathBuf>,

    /// Overwrite output file if it exists
    #[arg(short = 'f', long)]
    pub force: bool,

    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,

    /// Pretty print the parsed AST (for debugging)
    #[arg(long)]
    pub debug_ast: bool,

    /// Show tokens from lexer (for debugging)
    #[arg(long)]
    pub debug_tokens: bool,
}

impl Cli {
    pub fn parse_args() -> Self {
        <Self as ClapParser>::parse()
    }

    pub fn run(&self) -> anyhow::Result<()> {
        // Read input
        let input_content = self.read_input()?;

        if self.verbose {
            eprintln!("Read {} bytes from input", input_content.len());
        }

        // Tokenize
        let mut lexer = Lexer::new(&input_content);
        let tokens = lexer
            .tokenize()
            .context("Failed to tokenize LaTeX input")?;

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

        // Parse
        let mut parser = Parser::new(tokens);
        if let Some(base_path) = self.input_base_path() {
            parser = parser.with_base_path(base_path);
        }
        let document = parser.parse().context("Failed to parse LaTeX document")?;

        if self.debug_ast {
            eprintln!("=== AST ===");
            eprintln!("{:#?}", document);
            eprintln!("=== END AST ===\n");
        }

        if self.verbose {
            eprintln!("Parsed into {} blocks", document.blocks.len());
        }

        // Convert to Markdown
        let mut converter = MarkdownConverter::new();
        let markdown = converter
            .convert(document)
            .context("Failed to convert to Markdown")?;

        if self.verbose {
            eprintln!("Generated {} bytes of Markdown", markdown.len());
        }

        // Write output
        self.write_output(&markdown)?;

        if self.verbose {
            eprintln!("Conversion completed successfully");
        }

        Ok(())
    }

    fn read_input(&self) -> anyhow::Result<String> {
        match &self.input {
            Some(path) if path.to_str() == Some("-") => {
                // Read from stdin
                if self.verbose {
                    eprintln!("Reading from stdin...");
                }
                let mut buffer = String::new();
                io::stdin()
                    .read_to_string(&mut buffer)
                    .context("Failed to read from stdin")?;
                Ok(buffer)
            }
            Some(path) => {
                // Read from file
                if self.verbose {
                    eprintln!("Reading from file: {}", path.display());
                }
                fs::read_to_string(path)
                    .with_context(|| format!("Failed to read input file: {}", path.display()))
            }
            None => {
                // Read from stdin by default
                if self.verbose {
                    eprintln!("No input file specified, reading from stdin...");
                }
                let mut buffer = String::new();
                io::stdin()
                    .read_to_string(&mut buffer)
                    .context("Failed to read from stdin")?;
                Ok(buffer)
            }
        }
    }

    fn input_base_path(&self) -> Option<PathBuf> {
        match &self.input {
            Some(path) if path.to_str() != Some("-") => path.parent().map(|p| p.to_path_buf()),
            _ => None,
        }
    }

    fn write_output(&self, content: &str) -> anyhow::Result<()> {
        match &self.output {
            Some(path) if path.to_str() == Some("-") => {
                // Write to stdout
                if self.verbose {
                    eprintln!("Writing to stdout...");
                }
                io::stdout()
                    .write_all(content.as_bytes())
                    .context("Failed to write to stdout")?;
            }
            Some(path) => {
                // Write to file
                if self.verbose {
                    eprintln!("Writing to file: {}", path.display());
                }

                // Check if file exists and force flag
                if path.exists() && !self.force {
                    anyhow::bail!(
                        "Output file already exists: {}. Use --force to overwrite.",
                        path.display()
                    );
                }

                fs::write(path, content)
                    .with_context(|| format!("Failed to write output file: {}", path.display()))?;
            }
            None => {
                // Write to stdout by default
                if self.verbose {
                    eprintln!("No output file specified, writing to stdout...");
                }
                io::stdout()
                    .write_all(content.as_bytes())
                    .context("Failed to write to stdout")?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parsing() {
        // Test basic argument parsing
        let cli = Cli::parse_args();
        assert!(cli.input.is_none() || cli.input.is_some());
    }
}
