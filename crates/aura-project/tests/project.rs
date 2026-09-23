//! `aura-project` tests — manifest parsing, discovery, dep graph.

use std::path::{Path, PathBuf};

use aura_project::{DepSpec, MANIFEST_NAME, ProjectError, load};

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aura-proj-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn parses_minimal_manifest() {
    let m = aura_project::Manifest::parse("[package]\nname = \"app\"\n").unwrap();
    assert_eq!(m.package.name, "app");
    assert!(m.dependencies.is_empty());
}

#[test]
fn parses_path_dependencies() {
    let text = "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
                [dependencies]\nutil = { path = \"../util\" }\nextra = { path = \"../extra\" }\n";
    let m = aura_project::Manifest::parse(text).unwrap();
    assert_eq!(m.package.version.as_deref(), Some("0.1.0"));
    assert_eq!(m.dependencies.len(), 2);
    assert_eq!(
        m.dependencies["util"],
        DepSpec::Path(PathBuf::from("../util"))
    );
}

#[test]
fn rejects_missing_package_name() {
    assert_eq!(
        aura_project::Manifest::parse("[dependencies]\n"),
        Err(ProjectError::NoPackageName)
    );
    assert_eq!(
        aura_project::Manifest::parse("[package]\nname = \"\"\n"),
        Err(ProjectError::NoPackageName)
    );
}

#[test]
fn rejects_version_deps() {
    let text = "[package]\nname = \"a\"\n[dependencies]\nx = \"1.0\"\n";
    assert_eq!(
        aura_project::Manifest::parse(text),
        Err(ProjectError::UnsupportedDep("x".into()))
    );
}

#[test]
fn rejects_malformed_toml() {
    assert!(matches!(
        aura_project::Manifest::parse("[package\nname"),
        Err(ProjectError::Manifest(_))
    ));
}

#[test]
fn bare_file_is_single_unit_project() {
    let dir = tmp("bare");
    let src = dir.join("one.aura");
    write(&src, "fn main() -> i64 { 0 }\n");
    let p = load(&src).unwrap();
    assert_eq!(p.bin_name(), "one");
    assert!(p.manifest_path.is_none());
    assert_eq!(p.sources.len(), 1);
    assert!(p.sources[0].is_main);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn project_loads_main_and_dep() {
    let dir = tmp("multi");
    write(
        &dir.join("app/aura.toml"),
        "[package]\nname = \"app\"\n[dependencies]\nutil = { path = \"../util\" }\n",
    );
    write(&dir.join("app/src/main.aura"), "fn main() -> i64 { 0 }\n");
    write(&dir.join("util/aura.toml"), "[package]\nname = \"util\"\n");
    write(&dir.join("util/src/lib.aura"), "fn u() -> i64 { 1 }\n");

    let p = load(&dir.join("app")).unwrap();
    assert_eq!(p.bin_name(), "app");
    assert_eq!(p.sources.len(), 2);
    assert!(!p.sources[0].is_main, "dep first");
    assert_eq!(p.sources[0].package, "util");
    assert!(p.sources[1].is_main, "entry last");
    assert!(p.sources[1].path.ends_with("main.aura"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn discovery_walks_up() {
    let dir = tmp("walk");
    write(&dir.join("app/aura.toml"), "[package]\nname = \"app\"\n");
    write(&dir.join("app/src/main.aura"), "fn main() -> i64 { 0 }\n");
    let p = load(&dir.join("app/src")).unwrap();
    assert_eq!(p.bin_name(), "app");
    assert!(p.manifest_path.unwrap().ends_with(MANIFEST_NAME));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_main_is_an_error() {
    let dir = tmp("nomain");
    write(&dir.join("app/aura.toml"), "[package]\nname = \"app\"\n");
    let err = load(&dir.join("app")).unwrap_err();
    assert!(matches!(err, ProjectError::MissingEntry(_)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cyclic_deps_error() {
    let dir = tmp("cycle");
    write(
        &dir.join("a/aura.toml"),
        "[package]\nname = \"a\"\n[dependencies]\nb = { path = \"../b\" }\n",
    );
    write(&dir.join("a/src/main.aura"), "fn main() -> i64 { 0 }\n");
    write(&dir.join("a/src/lib.aura"), "fn x() -> i64 { 0 }\n");
    write(
        &dir.join("b/aura.toml"),
        "[package]\nname = \"b\"\n[dependencies]\na = { path = \"../a\" }\n",
    );
    write(&dir.join("b/src/lib.aura"), "fn y() -> i64 { 0 }\n");
    let err = load(&dir.join("a")).unwrap_err();
    assert!(matches!(err, ProjectError::Cycle(_)), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dep_name_must_match_manifest() {
    let dir = tmp("mismatch");
    write(
        &dir.join("app/aura.toml"),
        "[package]\nname = \"app\"\n[dependencies]\nalias = { path = \"../dep\" }\n",
    );
    write(&dir.join("app/src/main.aura"), "fn main() -> i64 { 0 }\n");
    write(&dir.join("dep/aura.toml"), "[package]\nname = \"real\"\n");
    write(&dir.join("dep/src/lib.aura"), "fn z() -> i64 { 0 }\n");
    let err = load(&dir.join("app")).unwrap_err();
    assert!(
        matches!(&err, ProjectError::NameMismatch { dep, found } if dep == "alias" && found == "real"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn transitive_deps_dedup() {
    // app -> [a, b]; a -> shared; b -> shared: shared loads once.
    let dir = tmp("diamond");
    write(
        &dir.join("app/aura.toml"),
        "[package]\nname = \"app\"\n[dependencies]\na = { path = \"../a\" }\nb = { path = \"../b\" }\n",
    );
    write(&dir.join("app/src/main.aura"), "fn main() -> i64 { 0 }\n");
    for d in ["a", "b"] {
        write(
            &dir.join(format!("{d}/aura.toml")),
            &format!(
                "[package]\nname = \"{d}\"\n[dependencies]\nshared = {{ path = \"../shared\" }}\n"
            ),
        );
        write(
            &dir.join(format!("{d}/src/lib.aura")),
            "fn z() -> i64 { 0 }\n",
        );
    }
    write(
        &dir.join("shared/aura.toml"),
        "[package]\nname = \"shared\"\n",
    );
    write(&dir.join("shared/src/lib.aura"), "fn s() -> i64 { 0 }\n");

    let p = load(&dir.join("app")).unwrap();
    let names: Vec<_> = p.sources.iter().map(|s| s.package.as_str()).collect();
    assert_eq!(names, ["shared", "a", "b", "app"], "DFS deps-first, dedup");
    let _ = std::fs::remove_dir_all(&dir);
}
