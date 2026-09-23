//! `aura.toml` project manifests and local path dependencies.
//!
//! A project is a directory containing `aura.toml`:
//!
//! ```toml
//! [package]
//! name = "hello"
//!
//! [dependencies]
//! util = { path = "../util" }
//! ```
//!
//! The entry point is `src/main.aura`; each path dependency contributes
//! `src/lib.aura`. Dependency items merge into one flat namespace
//! (alpha semantics — there is no module system yet).

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// Manifest file name searched for in project directories.
pub const MANIFEST_NAME: &str = "aura.toml";
/// Conventional entry point inside a package.
pub const MAIN_SRC: &str = "src/main.aura";
/// Conventional library root inside a package.
pub const LIB_SRC: &str = "src/lib.aura";

/// A parsed `aura.toml`.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    /// `[package]` table — `name` is required.
    pub package: Package,
    /// `[dependencies]` — path deps only for now.
    pub dependencies: BTreeMap<String, DepSpec>,
}

/// The `[package]` table.
#[derive(Debug, Clone, PartialEq)]
pub struct Package {
    /// Package name — also the default output binary name.
    pub name: String,
    /// Optional semantic version string.
    pub version: Option<String>,
}

/// One `[dependencies]` entry.
#[derive(Debug, Clone, PartialEq)]
pub enum DepSpec {
    /// `{ path = "../util" }` — a local package directory.
    Path(PathBuf),
}

/// A single compilation unit: one `.aura` file plus which package it
/// came from (for diagnostics).
#[derive(Debug, Clone, PartialEq)]
pub struct SourceUnit {
    /// Absolute path to the `.aura` file.
    pub path: PathBuf,
    /// Package this file belongs to (`""` for the root package).
    pub package: String,
    /// `true` for the project's entry point (`src/main.aura` or a bare
    /// `.aura` file given on the command line).
    pub is_main: bool,
}

/// A loaded project: manifest plus ordered source units (deps first —
/// they may not see each other, but the root sees them all).
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    /// Directory containing the root `aura.toml` (or the file's parent
    /// for bare-file invocations).
    pub root: PathBuf,
    /// Root manifest — synthesized for bare `.aura` invocations.
    pub manifest: Manifest,
    /// The `aura.toml` this project came from — `None` when `load` was
    /// given a bare `.aura` file.
    pub manifest_path: Option<PathBuf>,
    /// All units in compile order: transitive deps (DFS) then the entry.
    pub sources: Vec<SourceUnit>,
}

impl Project {
    /// The entry unit (always last).
    ///
    /// # Panics
    /// Never panics — `load` always pushes an entry unit.
    #[must_use]
    pub fn entry(&self) -> &SourceUnit {
        self.sources.last().expect("project has an entry")
    }

    /// Default output binary name — the package name.
    #[must_use]
    pub fn bin_name(&self) -> &str {
        &self.manifest.package.name
    }
}

/// Manifest/project load failures.
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectError {
    /// `aura.toml` parse error (message carries line:col from `toml`).
    Manifest(String),
    /// A required source file does not exist.
    MissingEntry(PathBuf),
    /// `[package] name` is missing or empty.
    NoPackageName,
    /// A path dependency's `aura.toml` is missing or unreadable.
    DepManifest(PathBuf),
    /// A path dependency's `src/lib.aura` is missing.
    DepEntry(PathBuf),
    /// Dependency name does not match the dep's `[package] name`.
    NameMismatch { dep: String, found: String },
    /// Cyclic path dependencies (cycle rendered as `a -> b -> a`).
    Cycle(String),
    /// Only path dependencies are supported in this alpha.
    UnsupportedDep(String),
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest(m) => write!(f, "invalid {MANIFEST_NAME}: {m}"),
            Self::MissingEntry(p) => write!(f, "entry point {} not found", p.display()),
            Self::NoPackageName => write!(f, "{MANIFEST_NAME}: [package] name is required"),
            Self::DepManifest(p) => {
                write!(
                    f,
                    "dependency manifest {} not found or invalid",
                    p.display()
                )
            }
            Self::DepEntry(p) => write!(f, "dependency entry {} not found", p.display()),
            Self::NameMismatch { dep, found } => write!(
                f,
                "dependency `{dep}` resolves to package `{found}` — names must match"
            ),
            Self::Cycle(c) => write!(f, "cyclic dependency: {c}"),
            Self::UnsupportedDep(d) => {
                write!(
                    f,
                    "dependency `{d}`: only `{{ path = \"..\" }}` deps are supported"
                )
            }
        }
    }
}

impl std::error::Error for ProjectError {}

// ---------- TOML schema -------------------------------------------------------

#[derive(serde::Deserialize)]
struct RawManifest {
    package: Option<RawPackage>,
    dependencies: Option<BTreeMap<String, RawDep>>,
}

#[derive(serde::Deserialize)]
struct RawPackage {
    name: Option<String>,
    version: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum RawDep {
    /// `dep = { path = "../x" }`
    Table { path: Option<String> },
    /// `dep = "1.0"` — registry deps, not yet supported.
    Version(#[allow(dead_code)] String),
}

impl Manifest {
    /// Parse `aura.toml` text.
    ///
    /// # Errors
    /// [`ProjectError::Manifest`] on malformed TOML or schema violations,
    /// [`ProjectError::NoPackageName`] when `[package] name` is absent.
    pub fn parse(text: &str) -> Result<Self, ProjectError> {
        let raw: RawManifest =
            toml::from_str(text).map_err(|e| ProjectError::Manifest(e.to_string()))?;
        let package = raw.package.ok_or(ProjectError::NoPackageName)?;
        let name = package
            .name
            .filter(|n| !n.trim().is_empty())
            .ok_or(ProjectError::NoPackageName)?;
        let mut dependencies = BTreeMap::new();
        for (name, dep) in raw.dependencies.unwrap_or_default() {
            match dep {
                RawDep::Table { path: Some(p) } => {
                    dependencies.insert(name, DepSpec::Path(PathBuf::from(p)));
                }
                RawDep::Table { path: None } | RawDep::Version(_) => {
                    return Err(ProjectError::UnsupportedDep(name));
                }
            }
        }
        Ok(Self {
            package: Package {
                name,
                version: package.version,
            },
            dependencies,
        })
    }
}

// ---------- discovery & loading ----------------------------------------------

/// Locate `aura.toml` starting at `dir`, walking up to the filesystem
/// root (cargo-style). Returns the manifest's directory.
#[must_use]
pub fn find_manifest_dir(dir: &Path) -> Option<PathBuf> {
    let mut d = Some(dir);
    while let Some(cur) = d {
        if cur.join(MANIFEST_NAME).is_file() {
            return Some(cur.to_path_buf());
        }
        d = cur.parent();
    }
    None
}

/// Load a project from a path that may be a manifest file, a directory,
/// or a bare `.aura` file.
///
/// # Errors
/// See [`ProjectError`].
pub fn load(path: &Path) -> Result<Project, ProjectError> {
    if path.is_dir() {
        // Canonicalize first: `".".parent()` is `""`, which would end the
        // walk-up at the CWD instead of climbing the real directory tree.
        let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let dir = find_manifest_dir(&canon)
            .ok_or_else(|| ProjectError::MissingEntry(canon.join(MANIFEST_NAME)))?;
        return load_manifest_dir(&dir);
    }
    if path.file_name().is_some_and(|n| n == MANIFEST_NAME) {
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        return load_manifest_dir(dir);
    }
    // Bare .aura file — single-unit pseudo-project.
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("aura")
        .to_owned();
    Ok(Project {
        root: path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
        manifest: Manifest {
            package: Package {
                name,
                version: None,
            },
            dependencies: BTreeMap::new(),
        },
        manifest_path: None,
        sources: vec![SourceUnit {
            path: path.to_path_buf(),
            package: String::new(),
            is_main: true,
        }],
    })
}

fn load_manifest_dir(dir: &Path) -> Result<Project, ProjectError> {
    let manifest_path = dir.join(MANIFEST_NAME);
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|_| ProjectError::MissingEntry(manifest_path.clone()))?;
    let manifest = Manifest::parse(&text)?;
    let mut sources = Vec::new();
    let mut visiting: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    collect_deps(&manifest, dir, &mut visiting, &mut seen, &mut sources)?;

    let entry = dir.join(MAIN_SRC);
    if !entry.is_file() {
        return Err(ProjectError::MissingEntry(entry));
    }
    sources.push(SourceUnit {
        path: entry,
        package: manifest.package.name.clone(),
        is_main: true,
    });
    Ok(Project {
        root: dir.to_path_buf(),
        manifest,
        manifest_path: Some(manifest_path),
        sources,
    })
}

/// DFS over path deps; pushes each dep's `src/lib.aura` into `sources`.
fn collect_deps(
    manifest: &Manifest,
    dir: &Path,
    visiting: &mut Vec<String>,
    seen: &mut Vec<String>,
    sources: &mut Vec<SourceUnit>,
) -> Result<(), ProjectError> {
    for (name, spec) in &manifest.dependencies {
        let DepSpec::Path(rel) = spec;
        if rel.as_os_str().is_empty() {
            return Err(ProjectError::UnsupportedDep(name.clone()));
        }
        let dep_dir = dir
            .join(rel)
            .canonicalize()
            .unwrap_or_else(|_| dir.join(rel));
        let dep_manifest_path = dep_dir.join(MANIFEST_NAME);
        let dep_text = std::fs::read_to_string(&dep_manifest_path)
            .map_err(|_| ProjectError::DepManifest(dep_manifest_path.clone()))?;
        let dep_manifest = Manifest::parse(&dep_text)
            .map_err(|_| ProjectError::DepManifest(dep_manifest_path.clone()))?;
        if dep_manifest.package.name != *name {
            return Err(ProjectError::NameMismatch {
                dep: name.clone(),
                found: dep_manifest.package.name,
            });
        }
        if visiting.contains(name) {
            let mut cyc = visiting.clone();
            cyc.push(name.clone());
            return Err(ProjectError::Cycle(cyc.join(" -> ")));
        }
        if seen.contains(name) {
            continue;
        }
        visiting.push(name.clone());
        collect_deps(&dep_manifest, &dep_dir, visiting, seen, sources)?;
        visiting.pop();
        seen.push(name.clone());

        let lib = dep_dir.join(LIB_SRC);
        if !lib.is_file() {
            return Err(ProjectError::DepEntry(lib));
        }
        sources.push(SourceUnit {
            path: lib,
            package: name.clone(),
            is_main: false,
        });
    }
    Ok(())
}
