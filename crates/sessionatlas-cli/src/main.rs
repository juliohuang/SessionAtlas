//! `sessionatlas` command-line entry point.
//!
//! Parses arguments with clap, wires real stdin/stdout/stderr into the
//! library's [`sessionatlas_cli::run`], and propagates the exit code.

use std::io;

use clap::Parser;

use sessionatlas_cli::cli::Cli;
use sessionatlas_cli::{run, Io};

fn main() {
    let cli = Cli::parse();
    let mut input = io::stdin().lock();
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    let mut io = Io {
        stdin: &mut input,
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let code = run(cli, &mut io);
    std::process::exit(code);
}
