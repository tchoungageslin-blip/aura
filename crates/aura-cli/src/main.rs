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
    /// Scaffold a new Aura project (aura.toml + src/main.aura).
    New {
        /// Project/directory name to create.
        name: PathBuf,
    },
    /// Print embedded documentation: a topic page, a builtin name,
    /// or an error code (`aura doc E2101`). Works offline.
    Doc {
        /// Topic, builtin name, or E-code (default: list topics).
        topic: Option<String>,
    },
    /// Open the Aura documentation (installed docs dir, else the site).
    Docs,
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
    /// Benchmark `main`: wall-clock `iters` runs of the compiled
    /// executable and of the reference interpreter, then report.
    Bench {
        path: Option<PathBuf>,
        /// Runs per engine (default 3).
        #[arg(short, long, default_value_t = 3)]
        iters: u32,
    },
    /// Run the validation testsuite: every `testsuite/<NN>-*` scenario
    /// in order, compiled and interpreted, stopping at the first failure.
    Test {
        /// testsuite root (default `./testsuite`) or a scenario dir.
        path: Option<PathBuf>,
        /// Run only scenarios whose name contains this string.
        #[arg(short, long)]
        only: Option<String>,
    },
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
        Command::New { name } => new(&name),
        Command::Doc { topic } => doc(topic.as_deref()),
        Command::Docs => docs(),
        Command::Check { path } => check(&or_cwd(path)),
        Command::Parse { path } => parse(&or_cwd(path)),
        Command::Mir { path } => mir(&or_cwd(path)),
        Command::Build { path, output } => build(&or_cwd(path), output.as_deref()),
        Command::Run { path } => run(&or_cwd(path)),
        Command::Lsp => ExitCode::from(u8::try_from(aura_lsp::serve()).unwrap_or(1)),
        Command::Interp { path } => interp(&or_cwd(path)),
        Command::Bench { path, iters } => bench(&or_cwd(path), iters),
        Command::Test { path, only } => test(path.as_deref(), only.as_deref()),
        Command::Fmt {
            path,
            check,
            stdout,
        } => fmt(&or_cwd(path), check, stdout),
    }
}

/// Documentation pages embedded into the binary — offline docs for
/// `aura doc`. The same files feed the website's mdBook build, so the
/// CLI can never drift from the published docs.
const DOC_PAGES: &[(&str, &str)] = &[
    ("intro", include_str!("../../../docs/src/intro.md")),
    (
        "getting-started",
        include_str!("../../../docs/src/getting-started.md"),
    ),
    ("demarrage", include_str!("../../../docs/src/fr/demarrage.md")),
    ("language", include_str!("../../../docs/src/language.md")),
    ("toolchain", include_str!("../../../docs/src/toolchain.md")),
    ("stdlib", include_str!("../../../docs/src/stdlib.md")),
    ("errors", include_str!("../../../docs/src/errors.md")),
    ("internals", include_str!("../../../docs/src/internals.md")),
];

/// `aura doc [topic]` — print an embedded doc page, a builtin's entry,
/// or an error-code explanation. `aura doc` alone lists topics.
fn doc(topic: Option<&str>) -> ExitCode {
    let Some(topic) = topic else {
        println!("Aura documentation topics:");
        for (name, _) in DOC_PAGES {
            println!("  {name}");
        }
        println!("\n`aura doc <topic>` · `aura doc vec_push` · `aura doc E2101`");
        return ExitCode::SUCCESS;
    };
    // Error codes: `aura doc E2101` → the `### E2101` section.
    let t = topic.to_ascii_uppercase();
    if t.len() == 5 && t.starts_with('E') && t[1..].chars().all(|c| c.is_ascii_digit()) {
        let errors = DOC_PAGES.iter().find(|(n, _)| *n == "errors").unwrap().1;
        let head = format!("### {t}");
        if let Some(start) = errors.find(&head) {
            let rest = &errors[start..];
            let end = rest[4..]
                .find("\n### ")
                .map(|i| i + 4)
                .or_else(|| rest.find("\n## "))
                .unwrap_or(rest.len());
            print!("{}", rest[..end].trim_end());
            println!();
            return ExitCode::SUCCESS;
        }
        eprintln!("error: no such error code `{t}` — see `aura doc errors`");
        return ExitCode::from(1);
    }
    // Page name → the whole page.
    if let Some((_, page)) = DOC_PAGES.iter().find(|(n, _)| *n == topic) {
        print!("{}", page.trim_end());
        println!();
        return ExitCode::SUCCESS;
    }
    // Builtin name → its canonical doc from aura_common::BuiltinFn.
    if let Some(b) = aura_common::BuiltinFn::by_name(topic) {
        println!("```aura\n{}\n```", b.doc());
        return ExitCode::SUCCESS;
    }
    eprintln!("error: no doc topic or builtin `{topic}` — see `aura doc`");
    ExitCode::from(1)
}

/// `aura docs` — open the installed docs dir (Inno layout), else the
/// bundled markdown dir, else the website.
fn docs() -> ExitCode {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(std::path::Path::to_path_buf));
    if let Some(d) = exe_dir {
        for cand in [d.join("docs"), d.join("docs/html")] {
            if cand.is_dir() {
                let _ = std::process::Command::new("cmd")
                    .args(["/c", "start", "", &cand.display().to_string()])
                    .spawn();
                println!("opened {}", cand.display());
                return ExitCode::SUCCESS;
            }
        }
    }
    println!("Docs: https://aura-lang.github.io/aura — or `aura doc <topic>` offline.");
    let _ = std::process::Command::new("cmd")
        .args(["/c", "start", "", "https://aura-lang.github.io/aura"])
        .spawn();
    ExitCode::SUCCESS
}

/// `aura new <name>` — scaffold `name/aura.toml` + `name/src/main.aura`.
fn new(name: &Path) -> ExitCode {
    let dir = name;
    let pkg = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let valid = !pkg.is_empty()
        && pkg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && pkg.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
    if !valid {
        eprintln!(
            "error: invalid package name `{pkg}` (letters, digits, `_`, `-`; must start with a letter)"
        );
        return ExitCode::from(1);
    }
    if dir.exists() && dir.read_dir().is_ok_and(|mut d| d.next().is_some()) {
        eprintln!("error: {} exists and is not empty", dir.display());
        return ExitCode::from(1);
    }
    let src = dir.join("src");
    if let Err(e) = std::fs::create_dir_all(&src) {
        eprintln!("error: cannot create {}: {e}", src.display());
        return ExitCode::from(1);
    }
    let manifest = format!("[package]\nname = \"{pkg}\"\n");
    let main = "fn main() -> i64 {\n    println(\"Hello, Aura!\")\n    return 0\n}\n";
    for (path, contents) in [
        (dir.join("aura.toml"), manifest.as_str()),
        (src.join("main.aura"), main),
        (dir.join(".gitignore"), "build/\n"),
    ] {
        if let Err(e) = std::fs::write(&path, contents) {
            eprintln!("error: cannot write {}: {e}", path.display());
            return ExitCode::from(1);
        }
    }
    println!("created `{pkg}` — cd {} && aura run", dir.display());
    ExitCode::SUCCESS
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
/// `AURA_RUNTIME_LIB` env → next to the executable (release packages) →
/// `runtime/target/{debug,release}` next to the executable's `target`
/// dir → same relative to the CWD.
fn runtime_lib() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("AURA_RUNTIME_LIB") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Ok(p);
        }
        return Err(format!("AURA_RUNTIME_LIB={} is not a file", p.display()));
    }
    let exe = std::env::current_exe().unwrap_or_default();
    if let Some(dir) = exe.parent() {
        let beside = dir.join("aura_runtime.lib");
        if beside.is_file() {
            return Ok(beside);
        }
    }
    let rel = [
        PathBuf::from("runtime/target/debug/aura_runtime.lib"),
        PathBuf::from("runtime/target/release/aura_runtime.lib"),
    ];
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

/// Compile `path` and link it into a fresh temp-dir executable.
/// Returns `(temp_dir, exe)` — removing `temp_dir` cleans both files.
fn link_temp_exe(obj_bytes: &[u8]) -> Option<(PathBuf, PathBuf)> {
    let dir = std::env::temp_dir().join(format!("aura-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let obj = dir.join("out.obj");
    let exe = dir.join(if cfg!(windows) { "out.exe" } else { "out" });
    if let Err(e) = std::fs::write(&obj, obj_bytes) {
        eprintln!("error: cannot write {}: {e}", obj.display());
        return None;
    }
    if link(&obj, &exe) == ExitCode::FAILURE {
        return None;
    }
    Some((dir, exe))
}

fn run(path: &Path) -> ExitCode {
    let Some((obj_bytes, _cache)) = compile(path) else {
        return ExitCode::FAILURE;
    };
    let Some((dir, exe)) = link_temp_exe(&obj_bytes) else {
        return ExitCode::FAILURE;
    };
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

fn bench(path: &Path, iters: u32) -> ExitCode {
    let iters = iters.max(1);
    let Some((obj_bytes, _cache)) = compile(path) else {
        return ExitCode::FAILURE;
    };
    let Some((dir, exe)) = link_temp_exe(&obj_bytes) else {
        return ExitCode::FAILURE;
    };
    let mut compiled: Vec<f64> = Vec::with_capacity(iters as usize);
    let mut compiled_code: Option<i32> = None;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        match std::process::Command::new(&exe).status() {
            Ok(s) => {
                compiled_code = s.code();
                compiled.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            Err(e) => {
                eprintln!("error: cannot run {}: {e}", exe.display());
                let _ = std::fs::remove_dir_all(&dir);
                return ExitCode::FAILURE;
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);

    // Interpreter runs in-process on the same project.
    let (db, project, cache) = match load_project(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    if render(check_project(&db, project).as_slice(), &cache) {
        return ExitCode::FAILURE;
    }
    let mut interp_ms: Vec<f64> = Vec::with_capacity(iters as usize);
    let mut interp_code: Option<i64> = None;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        match aura_interp::run_project(&db, project) {
            Ok(v) => {
                interp_code = Some(v);
                interp_ms.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    let stats = |ts: &[f64]| {
        let min = ts.iter().copied().fold(f64::INFINITY, f64::min);
        let n = u32::try_from(ts.len()).unwrap_or(u32::MAX).max(1);
        let avg = ts.iter().sum::<f64>() / f64::from(n);
        (min, avg)
    };
    let (cmin, cavg) = stats(&compiled);
    let (imin, iavg) = stats(&interp_ms);
    println!("bench {}", path.display());
    println!(
        "  compiled: {} runs, min {cmin:.1}ms avg {cavg:.1}ms (exit {:?})",
        compiled.len(),
        compiled_code
    );
    println!(
        "  interp:   {} runs, min {imin:.1}ms avg {iavg:.1}ms (exit {:?})",
        interp_ms.len(),
        interp_code
    );
    let agree = compiled_code.map(i64::from) == interp_code;
    println!("  exit codes {}", if agree { "agree" } else { "DIFFER" });
    if agree {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

// ----- aura test -------------------------------------------------------------

/// One testsuite directory: `<NN>-<name>/` holding a `main.aura` file or
/// a full `aura.toml` project, plus optional expectation files.
struct Scenario {
    dir: PathBuf,
    /// `expected.diag` — a code like `E2001` the check must produce
    /// (diagnostic-only scenario; nothing is compiled or run).
    diag: Option<String>,
    /// `expected.stdout` — exact stdout (CRLF-normalized, tail-trimmed).
    stdout: Option<String>,
    /// Per-engine stdout overrides.
    stdout_compiled: Option<String>,
    stdout_interp: Option<String>,
    /// `expected.exit` — default `0`.
    exit: i32,
    exit_compiled: Option<i32>,
    exit_interp: Option<i32>,
    /// `stdin.txt` — fed to the process's stdin.
    stdin: Vec<u8>,
    /// `args.txt` — whitespace-split argv.
    args: Vec<String>,
    /// `skip.interp` marker — compiled-only scenario.
    skip_interp: bool,
    /// `interp.error` marker — interpreter must fail (any `InterpError`).
    expect_interp_err: bool,
}

fn read_opt(dir: &Path, name: &str) -> Option<String> {
    let p = dir.join(name);
    std::fs::read_to_string(&p).ok()
}

fn read_marker(dir: &Path, name: &str) -> bool {
    dir.join(name).is_file()
}

fn norm(s: &str) -> String {
    s.replace("\r\n", "\n").trim_end().to_owned()
}

fn scenario(dir: &Path) -> Result<Scenario, String> {
    if !dir.join("aura.toml").is_file() && !dir.join("main.aura").is_file() {
        return Err(format!("{}: no main.aura or aura.toml", dir.display()));
    }
    let parse_i32 =
        |f: &str| -> Option<i32> { read_opt(dir, f).and_then(|s| s.trim().parse::<i32>().ok()) };
    Ok(Scenario {
        diag: read_opt(dir, "expected.diag").map(|s| norm(&s)),
        stdout: read_opt(dir, "expected.stdout").map(|s| norm(&s)),
        stdout_compiled: read_opt(dir, "expected.stdout.compiled").map(|s| norm(&s)),
        stdout_interp: read_opt(dir, "expected.stdout.interp").map(|s| norm(&s)),
        exit: parse_i32("expected.exit").unwrap_or(0),
        exit_compiled: parse_i32("expected.exit.compiled"),
        exit_interp: parse_i32("expected.exit.interp"),
        stdin: std::fs::read(dir.join("stdin.txt")).unwrap_or_default(),
        args: read_opt(dir, "args.txt")
            .map(|s| s.split_whitespace().map(str::to_owned).collect())
            .unwrap_or_default(),
        skip_interp: read_marker(dir, "skip.interp"),
        expect_interp_err: read_marker(dir, "interp.error"),
        dir: dir.to_path_buf(),
    })
}

/// Unique temp dir under the OS temp root.
fn tempdir(tag: &str) -> PathBuf {
    let mut d = std::env::temp_dir();
    d.push(format!("{tag}_{}", std::process::id()));
    d
}

/// Recursive directory copy — scenario dirs are a handful of small
/// files, so a naive read/write walk is fine.
fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let dst = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &dst)?;
        } else {
            std::fs::copy(e.path(), &dst)?;
        }
    }
    Ok(())
}

/// `want` vs `got` — first differing line index, for failure reports.
fn first_diff(want: &str, got: &str) -> String {
    for (i, (w, g)) in want.lines().zip(got.lines()).enumerate() {
        if w != g {
            return format!("line {}: want {w:?} got {g:?}", i + 1);
        }
    }
    format!(
        "length: want {} lines, got {} lines",
        want.lines().count(),
        got.lines().count()
    )
}

fn run_scenario(dir: &Path) -> Result<(), String> {
    let sc = scenario(dir)?;
    let source = if sc.dir.join("aura.toml").is_file() {
        sc.dir.clone()
    } else {
        sc.dir.join("main.aura")
    };
    let (db, project, cache) = load_project(&source)?;
    let diags = check_project(&db, project);

    // Diagnostic-only scenario: the check itself is the assertion.
    if let Some(code) = &sc.diag {
        if diags.iter().any(|d| d.code == Some(code.as_str())) {
            return Ok(());
        }
        return Err(format!(
            "{code} not produced ({} diagnostics emitted)",
            diags.len()
        ));
    }
    if render(diags.as_slice(), &cache) {
        return Err("semantic errors".into());
    }

    // Run inside a throwaway copy of the scenario dir — file side
    // effects (write_file/exec children) never touch the repo.
    let work = tempdir(&format!(
        "aura_test_{}",
        sc.dir.file_name().unwrap_or_default().to_string_lossy()
    ));
    copy_dir(&sc.dir, &work).map_err(|e| format!("scenario workdir: {e}"))?;

    // Compiled engine.
    let (obj_bytes, _c) = compile(&source).ok_or("compile failed")?;
    let (tmp, exe) = link_temp_exe(&obj_bytes).ok_or("link failed")?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.current_dir(&work)
        .args(&sc.args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("spawn: {e}"))?;
    if let Some(mut w) = child.stdin.take() {
        let _ = w.write_all(&sc.stdin);
    }
    let out = child.wait_with_output().map_err(|e| format!("wait: {e}"))?;
    let _ = std::fs::remove_dir_all(&tmp);

    let want_exit = sc.exit_compiled.unwrap_or(sc.exit);
    if out.status.code() != Some(want_exit) {
        return Err(format!(
            "compiled exit: want {want_exit}, got {:?}",
            out.status.code()
        ));
    }
    let want_out = sc.stdout_compiled.as_ref().or(sc.stdout.as_ref());
    if let Some(w) = want_out {
        let got = norm(&String::from_utf8_lossy(&out.stdout));
        if got != *w {
            return Err(format!("compiled stdout: {}", first_diff(w, &got)));
        }
    }

    // Interp engine.
    if sc.skip_interp {
        return Ok(());
    }
    // argv[0] is the spawned exe path compiled-side; interp uses its
    // own — scenarios must not depend on argv[0]'s value, only its
    // presence.
    let mut argv = vec!["aura-scenario".to_owned()];
    argv.extend(sc.args.iter().cloned());
    let cap = aura_interp::run_project_capture(
        &db,
        project,
        aura_interp::RunConfig {
            stdin: Some(sc.stdin.clone()),
            cwd: Some(work.clone()),
            args: Some(argv),
        },
    );
    let _ = std::fs::remove_dir_all(&work);
    match (&cap.result, sc.expect_interp_err) {
        (Err(_), true) => {}
        (Err(e), false) => return Err(format!("interp error: {e}")),
        (Ok(code), true) => return Err(format!("interp should error, got exit {code}")),
        (Ok(code), false) => {
            let want = sc.exit_interp.unwrap_or(sc.exit);
            if *code != i64::from(want) {
                return Err(format!("interp exit: want {want}, got {code}"));
            }
        }
    }
    let want_out = sc.stdout_interp.as_ref().or(sc.stdout.as_ref());
    if let Some(w) = want_out {
        let got = norm(&String::from_utf8_lossy(&cap.stdout));
        if got != *w {
            return Err(format!("interp stdout: {}", first_diff(w, &got)));
        }
    }
    Ok(())
}

fn test(path: Option<&Path>, only: Option<&str>) -> ExitCode {
    let root = path.map_or_else(|| PathBuf::from("testsuite"), Path::to_path_buf);
    let mut dirs: Vec<PathBuf> =
        if root.join("main.aura").is_file() || root.join("aura.toml").is_file() {
            vec![root.clone()]
        } else {
            match std::fs::read_dir(&root) {
                Ok(rd) => rd
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.is_dir())
                    .collect(),
                Err(e) => {
                    eprintln!("error: cannot read {}: {e}", root.display());
                    return ExitCode::FAILURE;
                }
            }
        };
    dirs.sort();
    if let Some(f) = only {
        dirs.retain(|d| {
            d.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains(f))
        });
    }
    if dirs.is_empty() {
        eprintln!("error: no scenarios found under {}", root.display());
        return ExitCode::FAILURE;
    }
    let total = dirs.len();
    let mut passed = 0;
    for dir in &dirs {
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_owned();
        match run_scenario(dir) {
            Ok(()) => {
                passed += 1;
                println!("ok   {name}");
            }
            Err(e) => {
                println!("FAIL {name}: {e}");
                println!("{passed}/{total} scenarios passed — stopped at first failure");
                return ExitCode::FAILURE;
            }
        }
    }
    println!("{passed}/{total} scenarios passed");
    ExitCode::SUCCESS
}
