use std::io::IsTerminal;
use texed::cli::Cli;

/// Stack size for the conversion thread. The parser bounds its own recursion
/// depth, but deeply nested documents still recurse a few hundred frames;
/// a generous stack keeps that far from the limit in debug and release.
const WORKER_STACK_SIZE: usize = 256 * 1024 * 1024;

fn main() {
    let cli = Cli::parse_args();
    let worker = std::thread::Builder::new()
        .name("texed".into())
        .stack_size(WORKER_STACK_SIZE)
        .spawn(move || cli.run());
    let result = match worker {
        Ok(handle) => handle
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("internal error: conversion thread panicked"))),
        Err(e) => Err(anyhow::anyhow!("failed to start conversion thread: {}", e)),
    };
    if let Err(error) = result {
        let red = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        let prefix = if red { "\x1b[1;31merror:\x1b[0m" } else { "error:" };
        eprintln!("{} {:#}", prefix, error);
        std::process::exit(1);
    }
}
