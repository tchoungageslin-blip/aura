//! Docs-as-code gate: every fenced `aura` block in docs/src/ must
//! compile clean; every `aura,fail` block must produce diagnostics —
//! and inside errors.md it must emit the code of its `### E####`
//! section. Docs can never silently drift from the compiler.

use aura_common::FileId;
use aura_salsa_db::{AuraDatabase, SourceFile};
use aura_semantic::check_file;
use std::path::{Path, PathBuf};

fn diags(src: &str) -> Vec<aura_common::Diagnostic> {
    let db = AuraDatabase::new();
    let file = SourceFile::new(&db, src.to_owned(), FileId(0));
    check_file(&db, file).clone()
}

/// Semantic diags; when clean, also codegen diags (E3xxx backstops
/// only fire once type checking passes).
fn all_diags(src: &str) -> Vec<aura_common::Diagnostic> {
    let db = AuraDatabase::new();
    let file = SourceFile::new(&db, src.to_owned(), FileId(0));
    let ds = check_file(&db, file).clone();
    if ds.iter().any(aura_common::Diagnostic::is_error) {
        return ds;
    }
    let mut out = ds;
    out.extend(aura_codegen::compile_file(&db, file).diagnostics);
    out
}

fn docs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/src")
}

fn md_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            md_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "md") {
            out.push(p);
        }
    }
}

/// True if any column-0 line is a statement/expression rather than an
/// item — meaning the block mixes items and loose statements.
fn has_loose_statements(block: &str) -> bool {
    block.lines().any(|l| {
        !l.starts_with(char::is_whitespace)
            && !l.is_empty()
            && !l.starts_with("//")
            && !l.starts_with("fn ")
            && !l.starts_with("struct ")
            && !l.starts_with("enum ")
            && !l.starts_with("use ")
            && !l.starts_with("extern ")
            && !l.starts_with('}')
            && !l.starts_with(')')
    })
}

fn is_item_block(block: &str) -> bool {
    block.lines().any(|l| {
        l.starts_with("fn ")
            || l.starts_with("struct ")
            || l.starts_with("enum ")
            || l.starts_with("use ")
            || l.starts_with("extern ")
    })
}

/// Blocks without top-level items are fragments: wrap them in main so
/// they are complete, compilable programs.
fn wrap_if_fragment(block: &str) -> String {
    if is_item_block(block) {
        assert!(
            !has_loose_statements(block),
            "mixed items + loose statements — split the block:\n{block}"
        );
        return block.to_owned();
    }
    format!("fn main() -> i64 {{\n{block}\n0\n}}\n")
}

#[test]
fn docs_examples_match_the_compiler() {
    let mut files = Vec::new();
    md_files(&docs_dir(), &mut files);
    files.sort();
    assert!(!files.is_empty(), "no docs found under docs/src");

    let mut checked = 0;
    let mut expected_fail = 0;
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        let mut current_code = String::new();
        let mut in_block = false;
        let mut fail_block = false;
        let mut ignore_block = false;
        let mut block = String::new();
        let mut block_start = 0usize;

        for (ln, line) in text.lines().enumerate() {
            if !in_block {
                if let Some(h) = line.strip_prefix("### E") {
                    current_code = format!("E{}", &h[..4.min(h.len())]);
                }
                if line.starts_with("```aura") {
                    in_block = true;
                    fail_block = line.contains(",fail");
                    ignore_block = line.contains(",ignore");
                    block.clear();
                    block_start = ln + 1;
                }
                continue;
            }
            if line.starts_with("```") {
                in_block = false;
                if ignore_block {
                    continue;
                }
                let src = wrap_if_fragment(&block);
                // Compile-blocks must type-check; fail-blocks go
                // through codegen too so E3xxx codes are reachable.
                let ds = if fail_block {
                    all_diags(&src)
                } else {
                    diags(&src)
                };
                let loc = format!("{}:{}", path.display(), block_start);
                if fail_block {
                    assert!(
                        !ds.is_empty(),
                        "{loc}: `aura,fail` block compiled cleanly:\n{block}"
                    );
                    expected_fail += 1;
                    // Inside errors.md a block must emit its section code.
                    if path.file_name().unwrap() == "errors.md" && !current_code.is_empty() {
                        assert!(
                            ds.iter().any(|d| d.code == Some(current_code.as_str())),
                            "{loc}: expected {current_code}, got {:?}:\n{block}",
                            ds.iter().filter_map(|d| d.code).collect::<Vec<_>>()
                        );
                    }
                } else {
                    assert!(
                        ds.is_empty(),
                        "{loc}: `aura` block has diagnostics {:?}:\n{block}",
                        ds.iter().map(|d| &d.message).collect::<Vec<_>>()
                    );
                    checked += 1;
                }
                continue;
            }
            block.push_str(line);
            block.push('\n');
        }
    }
    assert!(checked >= 10, "suspiciously few aura blocks: {checked}");
    assert!(expected_fail >= 10, "error index lost its fail blocks");
    eprintln!("docs gate: {checked} compile-clean, {expected_fail} expected-fail blocks");
}

/// errors.md must document every code defined in `codes::*` — the index
/// is complete or the test fails.
#[test]
fn error_index_covers_all_codes() {
    let errors_md = std::fs::read_to_string(docs_dir().join("errors.md")).unwrap();
    let common_src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../aura-common/src/lib.rs"),
    )
    .unwrap();
    for line in common_src.lines() {
        if let Some(rest) = line.split("= \"").nth(1)
            && let Some(code) = rest.strip_prefix('E')
            && code.len() >= 4
            && code[..4].chars().all(|c| c.is_ascii_digit())
        {
            let code = format!("E{}", &code[..4]);
            assert!(
                errors_md.contains(&format!("### {code}")),
                "errors.md is missing a section for {code}"
            );
        }
    }
}
