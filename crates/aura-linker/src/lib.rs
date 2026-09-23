//! aura-linker — platform-neutral final link step.
//!
//! One trait, three drivers:
//! - [`LldLink`] — Windows: `lld-link.exe` bundled inside the Rust
//!   toolchain (`rustlib/../gcc-ld/`), MSVC-flavour flags. The runtime is
//!   freestanding (`/nodefaultlib`), so no Windows SDK libs are needed.
//! - [`Mold`] — Linux/CI: `mold` from `PATH` (the megaplan's CI choice).
//! - [`Ld`] — generic `ld.lld`/`ld` fallback for other Unixes.
//!
//! `default_linker()` picks for the host OS. `AURA_LINKER` overrides for
//! testing (`lld-link`, `mold`, `ld`).

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What goes into one link.
#[derive(Debug)]
pub struct LinkRequest {
    /// Compiler-emitted object files (`.obj`/`.o`).
    pub objects: Vec<PathBuf>,
    /// Static archives (`aura_runtime.lib`, …).
    pub libs: Vec<PathBuf>,
    /// Output executable path.
    pub output: PathBuf,
    /// Entry symbol — `mainCRTStartup` on Windows, `_start`/`main` elsewhere.
    pub entry: Option<String>,
}

/// Everything that can go wrong locating or running a linker.
#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    /// No usable linker found on this system.
    #[error("no linker found: {0}")]
    NotFound(String),
    /// The linker ran but rejected the inputs.
    #[error("{driver} failed (exit {status:?}): {stderr}")]
    Failed {
        driver: &'static str,
        status: Option<i32>,
        stderr: String,
    },
    /// Spawning the linker failed.
    #[error("cannot spawn linker: {0}")]
    Spawn(io::Error),
}

/// A concrete linker we know how to drive.
pub trait LinkerDriver {
    /// Human/driver name for diagnostics.
    fn name(&self) -> &'static str;
    /// Perform the link.
    ///
    /// # Errors
    /// `Spawn` if the driver can't be exec'd; `Failed` if it exits nonzero.
    fn link(&self, req: &LinkRequest) -> Result<(), LinkError>;
    /// Where the driver binary lives (for diagnostics).
    fn path(&self) -> &Path;
}

// ----- Windows: LLD -------------------------------------------------------------

/// `lld-link` — MSVC-compatible LLD driver.
pub struct LldLink {
    path: PathBuf,
}

impl LldLink {
    /// The bundled `lld-link.exe` inside the active Rust sysroot.
    /// `rustc --print sysroot` → `lib/rustlib/<host>/bin/gcc-ld/lld-link.exe`.
    pub fn from_toolchain() -> Option<Self> {
        let out = Command::new("rustc")
            .arg("--print")
            .arg("sysroot")
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let sysroot = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        let candidate =
            Path::new(&sysroot).join("lib/rustlib/x86_64-pc-windows-msvc/bin/gcc-ld/lld-link.exe");
        if candidate.exists() {
            Some(Self { path: candidate })
        } else {
            None
        }
    }

    /// `lld-link`/`lld-link.exe` on `PATH`.
    pub fn from_path() -> Option<Self> {
        which("lld-link").map(|path| Self { path })
    }

    /// Scan `~/.rustup/toolchains/*/` directly — works even when neither
    /// `rustc` nor `lld-link` is on `PATH`.
    pub fn from_rustup_dir() -> Option<Self> {
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)?;
        let toolchains = home.join(".rustup/toolchains");
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(toolchains)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for tc in dirs {
            let candidate = tc.join("lib/rustlib/x86_64-pc-windows-msvc/bin/gcc-ld/lld-link.exe");
            if candidate.is_file() {
                return Some(Self { path: candidate });
            }
        }
        None
    }

    /// Any working `lld-link`.
    pub fn find() -> Option<Self> {
        Self::from_toolchain()
            .or_else(Self::from_path)
            .or_else(Self::from_rustup_dir)
    }

    /// A driver at an explicit path (testing, embedded toolchains).
    pub const fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// The full invocation for `req`.
    fn command(&self, req: &LinkRequest) -> Command {
        let mut cmd = Command::new(&self.path);
        // Freestanding: runtime supplies the entry + all imports itself.
        cmd.arg("/nodefaultlib");
        cmd.arg("/subsystem:console");
        if let Some(entry) = &req.entry {
            cmd.arg(format!("/entry:{entry}"));
        }
        cmd.arg(format!("/out:{}", req.output.display()));
        for o in &req.objects {
            cmd.arg(o);
        }
        for l in &req.libs {
            cmd.arg(l);
        }
        cmd
    }
}

impl LinkerDriver for LldLink {
    fn name(&self) -> &'static str {
        "lld-link"
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn link(&self, req: &LinkRequest) -> Result<(), LinkError> {
        run(self.command(req), self.name())
    }
}

// ----- Unix: Mold / ld ------------------------------------------------------------

/// `mold` — the megaplan's Linux/CI linker.
pub struct Mold {
    path: PathBuf,
}

impl Mold {
    /// `mold` on `PATH`.
    pub fn find() -> Option<Self> {
        which("mold").map(|path| Self { path })
    }

    /// A driver at an explicit path (testing, embedded toolchains).
    pub const fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// The full invocation for `req`.
    fn command(&self, req: &LinkRequest) -> Command {
        let mut cmd = Command::new(&self.path);
        cmd.arg("-o").arg(&req.output);
        if let Some(entry) = &req.entry {
            cmd.arg("-e").arg(entry);
        }
        for o in &req.objects {
            cmd.arg(o);
        }
        for l in &req.libs {
            cmd.arg(l);
        }
        cmd
    }
}

impl LinkerDriver for Mold {
    fn name(&self) -> &'static str {
        "mold"
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn link(&self, req: &LinkRequest) -> Result<(), LinkError> {
        run(self.command(req), self.name())
    }
}

/// Generic `ld.lld`/`ld` fallback.
pub struct Ld {
    path: PathBuf,
}

impl Ld {
    /// First of `ld.lld`, `ld` on `PATH`.
    pub fn find() -> Option<Self> {
        which("ld.lld")
            .or_else(|| which("ld"))
            .map(|path| Self { path })
    }

    /// A driver at an explicit path (testing, embedded toolchains).
    pub const fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// The full invocation for `req`.
    fn command(&self, req: &LinkRequest) -> Command {
        let mut cmd = Command::new(&self.path);
        cmd.arg("-o").arg(&req.output);
        if let Some(entry) = &req.entry {
            cmd.arg("-e").arg(entry);
        }
        for o in &req.objects {
            cmd.arg(o);
        }
        for l in &req.libs {
            cmd.arg(l);
        }
        cmd
    }
}

impl LinkerDriver for Ld {
    fn name(&self) -> &'static str {
        "ld"
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn link(&self, req: &LinkRequest) -> Result<(), LinkError> {
        run(self.command(req), self.name())
    }
}

// ----- selection ------------------------------------------------------------------

/// Pick the host-appropriate linker driver.
///
/// Windows → bundled `lld-link`; Linux → `mold`, falling back to `ld`;
/// `AURA_LINKER` env var forces a named driver (`lld-link`/`mold`/`ld`).
///
/// # Errors
/// `NotFound` if no driver exists for this host (or the `AURA_LINKER`
/// override names an unknown/missing driver).
pub fn default_linker() -> Result<Box<dyn LinkerDriver>, LinkError> {
    if let Ok(force) = std::env::var("AURA_LINKER") {
        return named(&force).ok_or_else(|| LinkError::NotFound(force.clone()));
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(l) = LldLink::find() {
            return Ok(Box::new(l));
        }
        Err(LinkError::NotFound(
            "lld-link (expected in the Rust sysroot or on PATH)".into(),
        ))
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(m) = Mold::find() {
            return Ok(Box::new(m));
        }
        if let Some(l) = Ld::find() {
            return Ok(Box::new(l));
        }
        return Err(LinkError::NotFound("mold or ld on PATH".into()));
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        if let Some(l) = Ld::find() {
            return Ok(Box::new(l));
        }
        Err(LinkError::NotFound("a system linker".into()))
    }
}

/// Resolve a driver by name for `AURA_LINKER`.
fn named(name: &str) -> Option<Box<dyn LinkerDriver>> {
    match name {
        "lld-link" => LldLink::find().map(|l| Box::new(l) as Box<dyn LinkerDriver>),
        "mold" => Mold::find().map(|m| Box::new(m) as Box<dyn LinkerDriver>),
        "ld" => Ld::find().map(|l| Box::new(l) as Box<dyn LinkerDriver>),
        _ => None,
    }
}

/// Look up `name` (+`.exe`) on `PATH` without pulling in a which-crate.
fn which(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        for cand in [name.to_owned(), format!("{name}.exe")] {
            let p = dir.join(cand);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn run(mut cmd: Command, driver: &'static str) -> Result<(), LinkError> {
    let out = cmd.output().map_err(LinkError::Spawn)?;
    if out.status.success() {
        Ok(())
    } else {
        Err(LinkError::Failed {
            driver,
            status: out.status.code(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> LinkRequest {
        LinkRequest {
            objects: vec![PathBuf::from("main.o")],
            libs: vec![PathBuf::from("aura_runtime.lib")],
            output: PathBuf::from("app.exe"),
            entry: Some("mainCRTStartup".into()),
        }
    }

    fn args(cmd: &Command) -> Vec<String> {
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn lld_link_command_shape() {
        let d = LldLink::at(PathBuf::from("lld-link.exe"));
        let a = args(&d.command(&req()));
        assert!(a.contains(&"/nodefaultlib".into()));
        assert!(a.contains(&"/subsystem:console".into()));
        assert!(a.contains(&"/entry:mainCRTStartup".into()));
        assert!(
            a.iter()
                .any(|x| x.starts_with("/out:") && x.contains("app.exe"))
        );
        assert!(a.contains(&"main.o".into()));
        assert!(a.contains(&"aura_runtime.lib".into()));
    }

    #[test]
    fn mold_command_shape() {
        let d = Mold::at(PathBuf::from("mold"));
        let a = args(&d.command(&req()));
        let o = a.iter().position(|x| x == "-o").unwrap();
        assert_eq!(a[o + 1], "app.exe");
        let e = a.iter().position(|x| x == "-e").unwrap();
        assert_eq!(a[e + 1], "mainCRTStartup");
        assert!(a.contains(&"main.o".into()));
        assert!(a.contains(&"aura_runtime.lib".into()));
    }

    #[test]
    fn ld_command_shape() {
        let d = Ld::at(PathBuf::from("ld"));
        let a = args(&d.command(&req()));
        assert_eq!(a[0], "-o");
        assert_eq!(a[1], "app.exe");
    }

    #[test]
    fn unknown_named_driver_is_none() {
        assert!(named("definitely-not-a-linker").is_none());
    }

    #[test]
    fn default_linker_finds_a_driver() {
        // Every supported host must yield some driver (dev machines have
        // the toolchain-bundled lld-link; CI has mold).
        match default_linker() {
            Ok(d) => assert!(!d.name().is_empty()),
            Err(e) => panic!("no linker on this machine: {e}"),
        }
    }

    #[test]
    fn missing_binary_is_spawn_error() {
        let d = Ld::at(PathBuf::from("Z:/nonexistent/ld-nothere"));
        let err = d.link(&req()).unwrap_err();
        assert!(matches!(err, LinkError::Spawn(_)));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn real_lld_link_rejects_bad_object() {
        let Some(d) = LldLink::find() else {
            eprintln!("lld-link not found — skipping");
            return;
        };
        let dir = std::env::temp_dir().join(format!("aura-link-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("bad.obj");
        std::fs::write(&bad, b"not a coff object").unwrap();
        let r = LinkRequest {
            objects: vec![bad],
            libs: vec![],
            output: dir.join("x.exe"),
            entry: None,
        };
        let err = d.link(&r).unwrap_err();
        assert!(matches!(err, LinkError::Failed { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn lld_link_found_on_this_machine() {
        // The dev image ships lld-link inside the Rust toolchain.
        assert!(LldLink::find().is_some());
    }
}
