//! End-to-end CLI tests — spawn the compiled `aura` binary.
//!
//! `build`/`run` tests need `runtime/target/debug/aura_runtime.lib`.
//! The runtime is a standalone package outside the workspace, so tests
//! build it on demand via `cargo`; if neither the lib nor cargo is
//! available, those tests report as skipped rather than failing.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const AURA: &str = env!("CARGO_BIN_EXE_aura");

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn fixture(name: &str) -> PathBuf {
    repo_root().join("fixtures").join(name)
}

fn aura(args: &[&str]) -> Output {
    Command::new(AURA)
        .args(args)
        .current_dir(repo_root())
        .output()
        .expect("spawn aura")
}

/// Ensure `aura_runtime.lib` exists; returns its path or `None` (skip).
fn runtime_lib() -> Option<PathBuf> {
    let root = repo_root();
    for profile in ["debug", "release"] {
        let lib = root.join(format!("runtime/target/{profile}/aura_runtime.lib"));
        if lib.is_file() {
            return Some(lib);
        }
    }
    let ok = Command::new("cargo")
        .args(["build", "--manifest-path"])
        .arg(root.join("runtime/Cargo.toml"))
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        let lib = root.join("runtime/target/debug/aura_runtime.lib");
        if lib.is_file() {
            return Some(lib);
        }
    }
    eprintln!("aura_runtime.lib unavailable — skipping link/run test");
    None
}

#[test]
fn check_hello_is_clean() {
    let out = aura(&["check", fixture("hello.aura").to_str().unwrap()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn check_bad_reports_errors() {
    let out = aura(&["check", fixture("bad.aura").to_str().unwrap()]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("E2"), "stderr: {stderr}");
}

#[test]
fn parse_dumps_sexpr() {
    let out = aura(&["parse", fixture("hello.aura").to_str().unwrap()]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("(fn"), "stdout: {stdout}");
}

#[test]
fn mir_dumps_both_functions() {
    let out = aura(&["mir", fixture("hello.aura").to_str().unwrap()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("fn length("), "stdout: {stdout}");
    assert!(stdout.contains("fn main("), "stdout: {stdout}");
    assert!(stdout.contains("bb0:"), "stdout: {stdout}");
}

#[test]
fn mir_rejects_bad_source() {
    let out = aura(&["mir", fixture("bad.aura").to_str().unwrap()]);
    assert!(!out.status.success());
}

#[test]
fn run_hello_returns_1() {
    if runtime_lib().is_none() {
        return;
    }
    let out = aura(&["run", fixture("hello.aura").to_str().unwrap()]);
    // hello.aura: length(Vec2{3,4}) = 5 > 0 → main returns 1.
    assert_eq!(
        out.status.code(),
        Some(1),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn run_enum_fixture_returns_12() {
    if runtime_lib().is_none() {
        return;
    }
    let out = aura(&["run", fixture("enum.aura").to_str().unwrap()]);
    // tag(Circle)*10 + tag(Rect) = 1*10 + 2 = 12.
    assert_eq!(
        out.status.code(),
        Some(12),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn run_result_fixture_returns_11() {
    if runtime_lib().is_none() {
        return;
    }
    let out = aura(&["run", fixture("result.aura").to_str().unwrap()]);
    // result.aura: outer(true) → inner(true)? = 10 → Ok(11) → match → 11.
    assert_eq!(
        out.status.code(),
        Some(11),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn run_strings_fixture_returns_6() {
    if runtime_lib().is_none() {
        return;
    }
    let out = aura(&["run", fixture("strings.aura").to_str().unwrap()]);
    // strings.aura: all str checks pass → tag(1) + 5 = 6.
    assert_eq!(
        out.status.code(),
        Some(6),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn run_vec_fixture_returns_11() {
    if runtime_lib().is_none() {
        return;
    }
    let out = aura(&["run", fixture("vec.aura").to_str().unwrap()]);
    // vec.aura: sum(10,99,30,40,50)=229, "ab"+"cd"="abcd", cap=8 → 11.
    assert_eq!(
        out.status.code(),
        Some(11),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn run_process_fixture_returns_13() {
    if runtime_lib().is_none() {
        return;
    }
    let out = aura(&["run", fixture("process.aura").to_str().unwrap()]);
    // process.aura: args() has >= 1 elem, PATH set, missing var is "" → 13.
    assert_eq!(
        out.status.code(),
        Some(13),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn run_print_fixture_outputs_hello_and_exits_3() {
    if runtime_lib().is_none() {
        return;
    }
    let out = aura(&["run", fixture("print.aura").to_str().unwrap()]);
    // print.aura: println → stdout, eprintln → stderr, exit(3).
    assert_eq!(
        out.status.code(),
        Some(3),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("Hello, World!"),
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("warn"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn run_result_err_path_returns_3() {
    if runtime_lib().is_none() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("aura-e2e-err-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("rerr.aura");
    std::fs::write(
        &src,
        "fn inner(ok: bool) -> Result<i64, i64> { if ok { Ok(10) } else { Err(3) } }\n\
         fn outer(ok: bool) -> Result<i64, i64> { let v = inner(ok)?\n Ok(v + 1) }\n\
         fn main() -> i64 { match outer(false) { Ok(v) => v, Err(e) => e } }\n",
    )
    .unwrap();
    let out = aura(&["run", src.to_str().unwrap()]);
    let _ = std::fs::remove_dir_all(&dir);
    // outer(false) → inner(false) = Err(3) → `?` propagates → match → 3.
    assert_eq!(
        out.status.code(),
        Some(3),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn fmt_rewrites_in_place_and_check_mode() {
    let dir = std::env::temp_dir().join(format!("aura-e2e-fmt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("ugly.aura");
    std::fs::write(&src, "fn   f(x:i64)->i64{x+1}\n").unwrap();
    // In-place rewrite.
    let out = aura(&["fmt", src.to_str().unwrap()]);
    assert!(out.status.success());
    let text = std::fs::read_to_string(&src).unwrap();
    assert_eq!(text, "fn f(x: i64) -> i64 {\n    x + 1\n}\n");
    // --check passes on formatted text, fails on unformatted.
    let ok = aura(&["fmt", "--check", src.to_str().unwrap()]);
    assert!(ok.status.success());
    std::fs::write(&src, "fn g()->i64{0}\n").unwrap();
    let dirty = aura(&["fmt", "--check", src.to_str().unwrap()]);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!dirty.status.success());
}

#[test]
fn fmt_refuses_parse_errors() {
    let dir = std::env::temp_dir().join(format!("aura-e2e-fmtbad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("bad.aura");
    std::fs::write(&src, "fn f( -> {}\n").unwrap();
    let out = aura(&["fmt", src.to_str().unwrap()]);
    // File must be untouched.
    assert_eq!(std::fs::read_to_string(&src).unwrap(), "fn f( -> {}\n");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!out.status.success());
}

#[test]
fn run_propagates_exit_code() {
    if runtime_lib().is_none() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("aura-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("ret42.aura");
    std::fs::write(&src, "fn main() -> i64 { 42 }\n").unwrap();
    let out = aura(&["run", src.to_str().unwrap()]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        out.status.code(),
        Some(42),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn build_produces_executable() {
    if runtime_lib().is_none() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("aura-e2e-build-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("hello_test.exe");
    let out = aura(&[
        "build",
        fixture("hello.aura").to_str().unwrap(),
        "-o",
        exe.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(exe.is_file(), "expected {} to exist", exe.display());
    let ran = Command::new(&exe).status().expect("run built exe");
    assert_eq!(ran.code(), Some(1));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_rejects_bad_source() {
    let out = aura(&["build", fixture("bad.aura").to_str().unwrap()]);
    assert!(!out.status.success());
}

/// Differential: `aura interp` (tree-walk reference) must agree with
/// `aura run` (Cranelift COFF) on the observable exit code.
#[test]
fn differential_interp_matches_compiled() {
    if runtime_lib().is_none() {
        return;
    }
    for name in [
        "hello.aura",
        "enum.aura",
        "result.aura",
        "strings.aura",
        "vec.aura",
        "process.aura",
    ] {
        let path = fixture(name);
        let path = path.to_str().unwrap();
        let compiled = aura(&["run", path]);
        let interp = aura(&["interp", path]);
        assert_eq!(
            compiled.status.code(),
            interp.status.code(),
            "{name}: compiled={:?} interp={:?} interp_stderr={}",
            compiled.status.code(),
            interp.status.code(),
            String::from_utf8_lossy(&interp.stderr)
        );
    }
}

#[test]
fn interp_reports_diagnostics_on_bad_source() {
    let out = aura(&["interp", fixture("bad.aura").to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("E2001"));
}

#[test]
fn interp_propagates_exit_code() {
    let dir = std::env::temp_dir().join(format!("aura-e2e-interp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("ret7.aura");
    std::fs::write(&src, "fn main() -> i64 { 7 }\n").unwrap();
    let out = aura(&["interp", src.to_str().unwrap()]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        out.status.code(),
        Some(7),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Write a two-package project (`app` depends on `util`) under `dir`.
fn write_project(dir: &Path) {
    std::fs::create_dir_all(dir.join("app/src")).unwrap();
    std::fs::create_dir_all(dir.join("util/src")).unwrap();
    std::fs::write(
        dir.join("app/aura.toml"),
        "[package]\nname = \"app\"\n\n[dependencies]\nutil = { path = \"../util\" }\n",
    )
    .unwrap();
    std::fs::write(dir.join("util/aura.toml"), "[package]\nname = \"util\"\n").unwrap();
    std::fs::write(
        dir.join("util/src/lib.aura"),
        "fn thrice(x: i64) -> i64 { x * 3 }\n\nstruct Pair { a: i64, b: i64 }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("app/src/main.aura"),
        "fn main() -> i64 {\n    let p = Pair { a: 10, b: 4 }\n    thrice(p.a) + p.b\n}\n",
    )
    .unwrap();
}

#[test]
fn project_check_interp_and_run() {
    let dir = std::env::temp_dir().join(format!("aura-e2e-proj-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    write_project(&dir);
    let app = dir.join("app");

    let out = aura(&["check", app.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Interp and compiled must agree on the cross-file program (34).
    let interp = aura(&["interp", app.to_str().unwrap()]);
    assert_eq!(interp.status.code(), Some(34));
    if runtime_lib().is_some() {
        let ran = aura(&["run", app.to_str().unwrap()]);
        assert_eq!(ran.status.code(), Some(34));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn project_dep_error_points_at_dep_file() {
    let dir = std::env::temp_dir().join(format!("aura-e2e-projerr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    write_project(&dir);
    // Break the dep: arithmetic on str is a semantic error in lib.aura.
    std::fs::write(
        dir.join("util/src/lib.aura"),
        "fn broken() -> i64 {\n    \"s\" + 1\n}\n",
    )
    .unwrap();
    let out = aura(&["check", dir.join("app").to_str().unwrap()]);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("lib.aura"),
        "dep error should attribute to lib.aura: {stderr}"
    );
}

#[test]
fn project_no_args_uses_cwd_manifest() {
    // `aura check` with no path discovers aura.toml from the CWD.
    let dir = std::env::temp_dir().join(format!("aura-e2e-projcwd-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    write_project(&dir);
    let out = Command::new(AURA)
        .args(["check"])
        .current_dir(dir.join("app"))
        .output()
        .expect("spawn aura");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
