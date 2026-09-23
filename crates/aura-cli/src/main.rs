//! `aura` — the Aura compiler CLI.
//!
//! Subcommands:
//! - `aura check <file.aura>` — parse, resolve, type-check; render diagnostics
//! - `aura parse <file.aura>` — dump the s-expression AST (debugging)

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use aura_common::{SourceCache, render_diagnostics};
use aura_salsa_db::{AuraDatabase, SourceFile, parsed};
use aura_semantic::check_file;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "aura", version, about = "The Aura compiler")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Parse, resolve, and type-check a source file.
    Check {
        /// Path to a `.aura` source file.
        path: PathBuf,
    },
    /// Parse a file and print its s-expression AST.
    Parse { path: PathBuf },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Check { path } => check(&path),
        Command::Parse { path } => parse(&path),
    }
}

fn load(path: &std::path::Path) -> Result<(AuraDatabase, SourceFile, SourceCache), std::io::Error> {
    let text = std::fs::read_to_string(path)?;
    let mut cache = SourceCache::new();
    let name = path.display().to_string();
    let file_id = cache.add(name, text.clone());
    let db = AuraDatabase::with_event_log(false);
    let file = SourceFile::new(&db, text, file_id);
    Ok((db, file, cache))
}

fn check(path: &Path) -> ExitCode {
    let (db, file, cache) = match load(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };
    let diags = check_file(&db, file);
    let stderr = std::io::stderr();
    let mut w = stderr.lock();
    let _ = render_diagnostics(diags, &cache, &mut w);
    let _ = w.flush();
    if diags.iter().any(aura_common::Diagnostic::is_error) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn parse(path: &Path) -> ExitCode {
    let (db, file, _cache) = match load(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };
    let p = parsed(&db, file);
    for line in p.dump().lines() {
        println!("{line}");
    }
    if p.diagnostics.iter().any(aura_common::Diagnostic::is_error) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
