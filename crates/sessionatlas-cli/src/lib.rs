//! `sessionatlas` CLI — testable command logic.
//!
//! This package builds a lib and a bin. The binary (`src/main.rs`) parses
//! arguments with clap and wires real stdin/stdout/stderr into [`run`]; every
//! command handler lives here so the read-only commands, `scan`, `config`, the
//! interactive selector and `open` are exercised against temporary databases
//! and config files in tests without ever touching the real `~/.sessionatlas`
//! or starting an external process.

pub mod cli;
pub mod commands;
pub mod db;
pub mod render;
pub mod select;

#[cfg(test)]
mod open_commands_tests;
#[cfg(test)]
mod readonly_commands_tests;
#[cfg(test)]
mod scan_config_commands_tests;

use std::path::{Path, PathBuf};

use cli::{Command, ListArgs};
use sessionatlas_core::launcher::default_wt_probe;
use sessionatlas_core::process::{PathProgramResolver, SystemProcessRunner};

use commands::open::OpenEnvironment;

/// IO bundle injected into every command handler so tests can script input and
/// capture output without a terminal.
pub struct Io<'io> {
    pub stdin: &'io mut dyn std::io::BufRead,
    pub stdout: &'io mut dyn std::io::Write,
    pub stderr: &'io mut dyn std::io::Write,
}

impl Io<'_> {
    /// Writes to stdout, ignoring transient write failures.
    pub fn out(&mut self, text: &str) {
        let _ = self.stdout.write_all(text.as_bytes());
        let _ = self.stdout.flush();
    }

    /// Writes to stderr, ignoring transient write failures.
    pub fn err(&mut self, text: &str) {
        let _ = self.stderr.write_all(text.as_bytes());
        let _ = self.stderr.flush();
    }
}

/// Runs the parsed CLI against the default database path
/// (`$SESSIONATLAS_HOME/.sessionatlas/index.db`).
pub fn run(cli: cli::Cli, io: &mut Io<'_>) -> i32 {
    let db_path = db::default_db_path();
    run_with_db(cli, io, &db_path)
}

/// Runs the parsed CLI against an explicit database path. Used by [`run`] and
/// by tests that point commands at an isolated temporary database.
pub fn run_with_db(cli: cli::Cli, io: &mut Io<'_>, db_path: &Path) -> i32 {
    let config_path = config_path_for_db(db_path);
    run_with_config(cli, io, db_path, &config_path)
}

/// Runs the parsed CLI against explicit database and config paths. `scan`
/// builds its scanner set from `config_path`; `config` reads and writes it.
pub fn run_with_config(cli: cli::Cli, io: &mut Io<'_>, db_path: &Path, config_path: &Path) -> i32 {
    let runner = SystemProcessRunner;
    let resolver = PathProgramResolver;
    let env = OpenEnvironment {
        runner: &runner,
        resolver: &resolver,
        wt_probe: default_wt_probe,
        resolved_session_id: None,
    };
    run_with_open_environment(cli, io, db_path, config_path, &env)
}

/// Runs the CLI with an injected launch environment. Tests use this entrypoint
/// for interactive list/search/recent flows without starting real processes.
pub fn run_with_open_environment(
    cli: cli::Cli,
    io: &mut Io<'_>,
    db_path: &Path,
    config_path: &Path,
    env: &OpenEnvironment<'_>,
) -> i32 {
    match cli.command {
        // No arguments: default interactive list, mirroring `Program.cs`.
        None => commands::list::run_list(
            io,
            db_path,
            config_path,
            &ListArgs {
                tool: None,
                limit: 50,
                interactive: true,
            },
            env,
        ),
        Some(Command::Scan(args)) => {
            let (scanners, diagnostics) = commands::scan::build_default_scanners(config_path);
            commands::scan::run_scan(io, db_path, &args, &scanners, &diagnostics)
        }
        Some(Command::List(args)) => commands::list::run_list(io, db_path, config_path, &args, env),
        Some(Command::Search(args)) => {
            commands::search::run_search(io, db_path, config_path, &args, env)
        }
        Some(Command::Open(args)) => commands::open::run_open(io, db_path, config_path, &args, env),
        Some(Command::Recent(args)) => {
            commands::recent::run_recent(io, db_path, config_path, &args, env)
        }
        Some(Command::Config(args)) => commands::config::run_config(io, config_path, &args),
    }
}

/// The config file that lives beside the index database
/// (`<db parent>/config.json`), so an explicit database path also pins its
/// config in isolation.
pub fn config_path_for_db(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("config.json")
}
