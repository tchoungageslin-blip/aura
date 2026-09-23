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
use aura_salsa_db::{AuraDatabase, Project, SourceFile, project_items};
use aura_semantic::check_project;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "aura", version, about = "The Aura compiler")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Parse, resolve, and type-check a source file or project.
    Check {
        /// `.aura` file, project dir, or manifest (default: `.`).
        path: Option<PathBuf>,
    },
    /// Parse a file and print its s-expression AST.
    Parse { path: Option<PathBuf> },
    /// Lower a file to MIR and print it (debugging).
    Mir { path: Option<PathBuf> },
    /// Compile and link a file or project into an executable.
    Build {
        path: Option<PathBuf>,
        /// Output executable path (default: `build/<package>` for
        /// projects, `<file>` + platform suffix otherwise).
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Compile, link, and run a file or project.
    Run { path: Option<PathBuf> },
    /// Interpret `main` directly (no codegen) and forward its exit code.
    Interp { path: Option<PathBuf> },
    /// Start the Language Server Protocol server over stdio.
    Lsp,
    /// Format a file or every source unit of a project canonically
    /// (in place unless --check/--stdout).
    Fmt {
        path: Option<PathBuf>,
        /// Exit non-zero if the file isn't already formatted.
        #[arg(long)]
        check: bool,
        /// Print the formatted source instead of rewriting the file.
        #[arg(long)]
        stdout: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let or_cwd = |p: Option<PathBuf>| p.unwrap_or_else(|| PathBuf::from("."));
    match cli.command {
        Command::Check { path } => check(&or_cwd(path)),
        Command::Parse { path } => parse(&or_cwd(path)),
        Command::Mir { path } => mir(&or_cwd(path)),
        Command::Build { path, output } => build(&or_cwd(path), output.as_deref()),
        Command::Run { path } => run(&or_cwd(path)),
        Command::Lsp => ExitCode::from(u8::try_from(aura_lsp::serve()).unwrap_or(1)),
        Command::Interp { path } => interp(&or_cwd(path)),
        Command::Fmt {
            path,
            check,
            stdout,
        } => fmt(&or_cwd(path), check, stdout),
    }
}

/// Resolve `path` (a `.aura` file, a directory containing `aura.toml`, or
/// a manifest path) into a salsa `Project` plus a `SourceCache` covering
/// every unit, so diagnostics attribute to real paths.
fn load_project(path: &Path) -> Result<(AuraDatabase, Project, SourceCache), String> {
    let spec = aura_project::load(path).map_err(|e| e.to_string())?;
    let mut cache = SourceCache::new();
    let db = AuraDatabase::with_event_log(false);
    let mut files = Vec::new();
    for unit in &spec.sources {
        let text = std::fs::read_to_string(&unit.path)
            .map_err(|e| format!("cannot read {}: {e}", unit.path.display()))?;
        let file_id = cache.add(unit.path.display().to_string(), text.clone());
        files.push(SourceFile::new(&db, text, file_id));
    }
    let project = Project::new(&db, files);
    Ok((db, project, cache))
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
    let (db, project, cache) = match load_project(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let diags = check_project(&db, project);
    if render(diags.as_slice(), &cache) {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn interp(path: &Path) -> ExitCode {
    let (db, project, cache) = match load_project(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let diags = check_project(&db, project);
    if render(diags.as_slice(), &cache) {
        return ExitCode::FAILURE;
    }
    match aura_interp::run_project(&db, project) {
        Ok(v) => ExitCode::from(u8::try_from(v).unwrap_or(1)),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parse(path: &Path) -> ExitCode {
    let (db, project, _cache) = match load_project(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Dump the entry unit (always last).
    let entry = *project.files(&db).last().expect("project has an entry");
    let p = aura_salsa_db::parsed(&db, entry);
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
    let (db, project, cache) = match load_project(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    if render(check_project(&db, project), &cache) {
        return ExitCode::FAILURE;
    }
    let items = &project_items(&db, project).merged;
    for (g, sig) in items.iter() {
        if !matches!(sig, aura_salsa_db::ItemSig::Fn { .. }) {
            continue;
        }
        if let Some(m) = aura_mir::mir_project_fn(&db, project, g).as_ref() {
            print!("{}", aura_mir::dump(m));
        }
    }
    ExitCode::SUCCESS
}

/// Compile to an object; render any diagnostics. `None` on failure.
fn compile(path: &Path) -> Option<(Vec<u8>, SourceCache)> {
    let (db, project, cache) = match load_project(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return None;
        }
    };
    if render(check_project(&db, project), &cache) {
        return None;
    }
    let out = aura_codegen::compile_project(&db, project);
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

/// Default output path: `<project-root>/build/<package>[.exe]` for
/// manifest projects, `<file>[.exe]` beside a bare source file.
fn default_output(path: &Path) -> PathBuf {
    let ext = if cfg!(windows) { "exe" } else { "" };
    if let Ok(spec) = aura_project::load(path)
        && spec.manifest_path.is_some()
    {
        let dir = spec.root.join("build");
        let _ = std::fs::create_dir_all(&dir);
        return dir.join(&spec.manifest.package.name).with_extension(ext);
    }
    let mut out = path.to_path_buf();
    out.set_extension(ext);
    out
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
    // Default output: <project>/build/<package>[.exe], or <stem>[.exe]
    // beside a bare source file.
    let out = output.map_or_else(|| default_output(path), Path::to_path_buf);
    let obj = out.with_extension("o");
    if let Err(e) = std::fs::write(&obj, &obj_bytes) {
        eprintln!("error: cannot write {}: {e}", obj.display());
        return ExitCode::FAILURE;
    }
    link(&obj, &out)
}

fn fmt(path: &Path, check: bool, to_stdout: bool) -> ExitCode {
    if path.is_dir() {
        // Project mode: format every source unit (deps first).
        let spec = match aura_project::load(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        };
        let mut failed = false;
        for unit in &spec.sources {
            if fmt_one(&unit.path, check, to_stdout) == ExitCode::FAILURE {
                failed = true;
            }
        }
        return if failed {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        };
    }
    fmt_one(path, check, to_stdout)
}

fn fmt_one(path: &Path, check: bool, to_stdout: bool) -> ExitCode {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };
    let formatted = match aura_fmt::format_source(&text) {
        Ok(f) => f,
        Err(diags) => {
            let mut cache = SourceCache::new();
            cache.add(path.display().to_string(), text);
            eprintln!("error: cannot format {} — parse errors:", path.display());
            render(&diags, &cache);
            return ExitCode::FAILURE;
        }
    };
    if check {
        if formatted == text {
            return ExitCode::SUCCESS;
        }
        eprintln!("error: {} is not formatted", path.display());
        return ExitCode::FAILURE;
    }
    if to_stdout {
        print!("{formatted}");
        return ExitCode::SUCCESS;
    }
    match std::fs::write(path, formatted) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: cannot write {}: {e}", path.display());
            ExitCode::FAILURE
        }
    }
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
