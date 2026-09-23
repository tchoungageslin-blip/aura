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
