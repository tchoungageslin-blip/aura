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
fn i128_literal_blocks_codegen() {
    let db = AuraDatabase::with_event_log(false);
    let file = SourceFile::new(
        &db,
        "fn main() -> i64 { let s: i128 = 1\n 0 }".to_owned(),
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

#[test]
fn result_and_try_emit_object() {
    let out = compile(
        "fn inner(ok: bool) -> Result<i64, i64> { if ok { Ok(1) } else { Err(2) } }\n\
         fn outer(ok: bool) -> Result<i64, i64> { let v = inner(ok)?\n Ok(v + 1) }\n\
         fn main() -> i64 { match outer(true) { Ok(v) => v, Err(e) => e } }",
    );
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    assert!(out.object.is_some());
}

#[test]
fn result_param_and_struct_field_emit() {
    let out = compile(
        "struct B { r: Result<i64, i64> }\n\
         fn unwrap(r: Result<i64, i64>) -> i64 { match r { Ok(v) => v, Err(e) => e } }\n\
         fn main() -> i64 { let b = B { r: Ok(9) }\n unwrap(b.r) }",
    );
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    assert!(out.object.is_some());
}

#[test]
fn str_literal_emits_object() {
    let out = compile("fn main() -> i64 { let s = \"hello\"\n if s.len == 5 { 1 } else { 0 } }");
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    assert!(out.object.is_some());
}

#[test]
fn str_equality_imports_aura_str_eq() {
    let out = compile("fn main() -> i64 { if \"a\" == \"a\" { 1 } else { 0 } }");
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let obj = out.object.expect("object expected");
    assert!(
        obj.windows(11).any(|w| w == b"aura_str_eq"),
        "object must import aura_str_eq"
    );
}

#[test]
fn str_param_return_and_struct_field_emit() {
    let out = compile(
        "struct P { s: str }\n\
         fn id(x: str) -> str { x }\n\
         fn main() -> i64 { let p = P { s: \"ab\" }\n if id(p.s) == \"ab\" { 3 } else { 0 } }",
    );
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    assert!(out.object.is_some());
}

#[test]
fn builtin_print_imports_aura_rt_println() {
    let out = compile("fn main() -> i64 { println(\"hi\")\n 0 }");
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let obj = out.object.expect("object expected");
    assert!(
        obj.windows(b"aura_rt_println".len())
            .any(|w| w == b"aura_rt_println"),
        "object must import aura_rt_println"
    );
}

#[test]
fn builtin_sqrt_imports_sqrt() {
    let out = compile("fn main() -> i64 { if sqrt(4.0) == 2.0 { 1 } else { 0 } }");
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let obj = out.object.expect("object expected");
    assert!(
        obj.windows(4).any(|w| w == b"sqrt"),
        "object must import sqrt"
    );
}

#[test]
fn str_add_imports_aura_str_concat() {
    let out =
        compile("fn main() -> i64 { let s = \"a\" + \"b\"\n if s == \"ab\" { 1 } else { 0 } }");
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let obj = out.object.expect("object expected");
    assert!(
        obj.windows(b"aura_str_concat".len())
            .any(|w| w == b"aura_str_concat"),
        "object must import aura_str_concat"
    );
}

#[test]
fn vec_ops_import_aura_vec_helpers() {
    let out = compile(
        "fn main() -> i64 { let v = vec_new()\n vec_push(v, 1)\n vec_set(v, 0, 2)\n vec_get(v, 0) }",
    );
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let obj = out.object.expect("object expected");
    assert!(
        obj.windows(b"aura_vec_push".len())
            .any(|w| w == b"aura_vec_push"),
        "object must import aura_vec_push"
    );
    assert!(
        obj.windows(b"aura_vec_get".len())
            .any(|w| w == b"aura_vec_get"),
        "object must import aura_vec_get"
    );
}

#[test]
fn args_env_import_runtime_symbols() {
    let out = compile(
        "fn main() -> i64 { let a = args()\n let e = env(\"PATH\")\n if a.len >= 1 && e.len > 0 { 1 } else { 0 } }",
    );
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let obj = out.object.expect("object expected");
    assert!(
        obj.windows(b"aura_rt_args".len())
            .any(|w| w == b"aura_rt_args"),
        "object must import aura_rt_args"
    );
    assert!(
        obj.windows(b"aura_rt_env".len())
            .any(|w| w == b"aura_rt_env"),
        "object must import aura_rt_env"
    );
}
