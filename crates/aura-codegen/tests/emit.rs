//! Codegen tests — object emission, symbol table, diagnostics.

use aura_codegen::compile_file;
use aura_common::FileId;
use aura_salsa_db::{AuraDatabase, SourceFile};
use aura_semantic::check_file;
use object::{Object, ObjectSymbol};

fn compile(src: &str) -> aura_codegen::CompileOutput {
    let db = AuraDatabase::with_event_log(false);
    let file = SourceFile::new(&db, src.to_owned(), FileId(0));
    let diags = check_file(&db, file);
    assert!(
        diags.iter().all(|d| !d.is_error()),
        "source must be clean: {diags:?}"
    );
    compile_file(&db, file)
}

#[test]
fn emits_valid_object_with_main() {
    let out = compile("fn main() -> i64 { 42 }");
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let bytes = out.object.expect("object expected");
    #[cfg(target_os = "windows")]
    assert_eq!(&bytes[0..2], &[0x64, 0x86]); // COFF machine x64
    #[cfg(target_os = "linux")]
    assert_eq!(&bytes[0..4], b"\x7fELF");

    // `main` must be an exported symbol.
    let obj = object::File::parse(&*bytes).expect("parse object");
    assert!(
        obj.symbols()
            .any(|s| s.name() == Ok("main") && s.is_global())
    );
}

#[test]
fn struct_params_and_calls_emit() {
    let out = compile(
        "struct V { x: f64, y: f64 }\n\
         fn len(v: V) -> f64 { v.x * v.x + v.y * v.y }\n\
         fn main() -> i64 { let p = V { x: 3.0, y: 4.0 }\n if len(p) > 0.0 { 1 } else { 0 } }",
    );
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    assert!(out.object.is_some());
}

#[test]
fn missing_main_is_e3005() {
    let db = AuraDatabase::with_event_log(false);
    let file = SourceFile::new(&db, "fn f() -> i64 { 1 }".to_owned(), FileId(0));
    let out = compile_file(&db, file);
    assert!(out.object.is_none());
    assert!(
        out.diagnostics
            .iter()
            .any(|d| d.code == Some(aura_common::codes::CG_MAIN_TYPE))
    );
}

#[test]
fn bad_main_sig_is_e3005() {
    let db = AuraDatabase::with_event_log(false);
    let file = SourceFile::new(&db, "fn main(x: i64) -> i64 { x }".to_owned(), FileId(0));
    let out = compile_file(&db, file);
    assert!(out.object.is_none());
    assert!(
        out.diagnostics
            .iter()
            .any(|d| d.code == Some(aura_common::codes::CG_MAIN_TYPE))
    );
}

#[test]
fn enum_match_emits_object() {
    let out = compile(
        "enum S { A(i64), B }\n\
         fn f(s: S) -> i64 { match s { A(x) => x, B => 0 } }\n\
         fn main() -> i64 { f(A(5)) }",
    );
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    assert!(out.object.is_some());
}

#[test]
fn str_literal_blocks_codegen() {
    let db = AuraDatabase::with_event_log(false);
    let file = SourceFile::new(
        &db,
        "fn main() -> i64 { let s = \"hi\"\n 0 }".to_owned(),
        FileId(0),
    );
    let out = compile_file(&db, file);
    assert!(out.object.is_none());
    assert!(
        out.diagnostics
            .iter()
            .any(|d| d.code == Some(aura_common::codes::CG_UNSUPPORTED))
    );
}
