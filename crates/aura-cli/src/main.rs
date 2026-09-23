//! `aura` — the Aura compiler CLI.
//!
//! Subcommands:
//! - `aura check <file.aura>` — parse, resolve, type-check; render diagnostics
//! - `aura parse <file.aura>` — dump the s-expression AST (debugging)
//! - `aura mir <file.aura>` — dump MIR for every function (debugging)
//! - `aura build <file.aura> [-o out]` — compile + link to an executable
//! - `aura run <file.aura>` — compile, link, execute, forward the exit code

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use aura_common::{SourceCache, render_diagnostics};
use aura_salsa_db::{AuraDatabase, SourceFile, file_items, parsed};
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
    /// Lower a file to MIR and print it (debugging).
    Mir { path: PathBuf },
    /// Compile and link a file into an executable.
    Build {
        path: PathBuf,
        /// Output executable path (default: `<file>` + platform suffix).
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Compile, link, and run a file.
    Run { path: PathBuf },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Check { path } => check(&path),
        Command::Parse { path } => parse(&path),
        Command::Mir { path } => mir(&path),
        Command::Build { path, output } => build(&path, output.as_deref()),
        Command::Run { path } => run(&path),
    }
}

fn load(path: &Path) -> Result<(AuraDatabase, SourceFile, SourceCache), std::io::Error> {
    let text = std::fs::read_to_string(path)?;
    let mut cache = SourceCache::new();
    let name = path.display().to_string();
    let file_id = cache.add(name, text.clone());
    let db = AuraDatabase::with_event_log(false);
    let file = SourceFile::new(&db, text, file_id);
    Ok((db, file, cache))
}

fn render(diags: &[aura_common::Diagnostic], cache: &SourceCache) -> bool {
    let stderr = std::io::stderr();
    let mut w = stderr.lock();
    let has_err = diags.iter().any(aura_common::Diagnostic::is_error);
    let _ = render_diagnostics(diags, cache, &mut w);
    let _ = w.flush();
    has_err
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
    if render(diags.as_slice(), &cache) {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
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

fn mir(path: &Path) -> ExitCode {
    let (db, file, cache) = match load(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };
    if render(check_file(&db, file), &cache) {
        return ExitCode::FAILURE;
    }
    let items = file_items(&db, file);
    for (i, sig) in items.iter() {
        if !matches!(sig, aura_salsa_db::ItemSig::Fn { .. }) {
            continue;
        }
        if let Some(m) = aura_mir::mir_fn(&db, file, i).as_ref() {
            print!("{}", aura_mir::dump(m));
        }
    }
    ExitCode::SUCCESS
}

/// Compile to an object; render any diagnostics. `None` on failure.
fn compile(path: &Path) -> Option<(Vec<u8>, SourceCache)> {
    let (db, file, cache) = match load(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", path.display());
            return None;
        }
    };
    if render(check_file(&db, file), &cache) {
        return None;
    }
    let out = aura_codegen::compile_file(&db, file);
    if render(&out.diagnostics, &cache) {
        return None;
    }
    out.object.map(|o| (o, cache))
}

/// Locate the prebuilt `aura_runtime.lib`. Search order:
/// `AURA_RUNTIME_LIB` env → `runtime/target/{debug,release}` next to the
/// executable's `target` dir → same relative to the CWD.
fn runtime_lib() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("AURA_RUNTIME_LIB") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Ok(p);
        }
        return Err(format!("AURA_RUNTIME_LIB={} is not a file", p.display()));
    }
    let rel = [
        PathBuf::from("runtime/target/debug/aura_runtime.lib"),
        PathBuf::from("runtime/target/release/aura_runtime.lib"),
    ];
    let exe = std::env::current_exe().unwrap_or_default();
    // exe is <repo>/target/{debug,release}/aura.exe → repo root is ../..
    for anchor in [
        exe.parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .map(Path::to_path_buf),
        std::env::current_dir().ok(),
    ]
    .into_iter()
    .flatten()
    {
        for r in &rel {
            let cand = anchor.join(r);
            if cand.is_file() {
                return Ok(cand);
            }
        }
    }
    Err("aura_runtime.lib not found — build it with `cargo build --manifest-path runtime/Cargo.toml`".into())
}

fn link(obj: &Path, out: &Path) -> ExitCode {
    let rt = match runtime_lib() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let driver = match aura_linker::default_linker() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let req = aura_linker::LinkRequest {
        objects: vec![obj.to_path_buf()],
        libs: vec![rt],
        output: out.to_path_buf(),
        entry: Some("mainCRTStartup".into()),
    };
    match driver.link(&req) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn build(path: &Path, output: Option<&Path>) -> ExitCode {
    let Some((obj_bytes, _cache)) = compile(path) else {
        return ExitCode::FAILURE;
    };
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let out = output.map_or_else(
        || {
            let mut p = PathBuf::from(stem.as_ref());
            p.set_extension(if cfg!(windows) { "exe" } else { "" });
            p
        },
        Path::to_path_buf,
    );
    let obj = out.with_extension("o");
    if let Err(e) = std::fs::write(&obj, &obj_bytes) {
        eprintln!("error: cannot write {}: {e}", obj.display());
        return ExitCode::FAILURE;
    }
    link(&obj, &out)
}

fn run(path: &Path) -> ExitCode {
    let Some((obj_bytes, _cache)) = compile(path) else {
        return ExitCode::FAILURE;
    };
    // Link under a temp dir; clean up best-effort.
    let dir = std::env::temp_dir().join(format!("aura-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let obj = dir.join("out.obj");
    let exe = dir.join(if cfg!(windows) { "out.exe" } else { "out" });
    if let Err(e) = std::fs::write(&obj, &obj_bytes) {
        eprintln!("error: cannot write {}: {e}", obj.display());
        return ExitCode::FAILURE;
    }
    if link(&obj, &exe) == ExitCode::FAILURE {
        return ExitCode::FAILURE;
    }
    let status = std::process::Command::new(&exe).status();
    let _ = std::fs::remove_dir_all(&dir);
    match status {
        Ok(s) => s.code().map_or(ExitCode::FAILURE, |c| {
            ExitCode::from(u8::try_from(c).unwrap_or(u8::MAX))
        }),
        Err(e) => {
            eprintln!("error: cannot run {}: {e}", exe.display());
            ExitCode::FAILURE
        }
    }
}
