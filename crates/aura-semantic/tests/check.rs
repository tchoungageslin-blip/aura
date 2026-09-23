//! Semantic tests: diagnostics per error code + salsa early-cutoff proof.

use aura_common::{FileId, codes};
use aura_salsa_db::{AuraDatabase, SourceFile};
use aura_semantic::check_file;
use salsa::Setter;

fn diags(src: &str) -> Vec<aura_common::Diagnostic> {
    let db = AuraDatabase::new();
    let file = SourceFile::new(&db, src.to_owned(), FileId(0));
    check_file(&db, file).clone()
}

fn codes_of(src: &str) -> Vec<&'static str> {
    diags(src).iter().filter_map(|d| d.code).collect()
}

fn assert_has(src: &str, code: &str) {
    let cs = codes_of(src);
    assert!(cs.contains(&code), "expected {code} in {cs:?} for:\n{src}");
}

// ----- clean programs ----------------------------------------------------------

#[test]
fn clean_program_no_diagnostics() {
    let src = "\
struct Point { x: f64, y: f64 }
fn dist(x: f64, y: f64) -> f64 {
    x * x + y * y
}
fn main() -> i64 {
    let p = Point { x: 3.0, y: 4.0 }
    let d = dist(p.x, p.y)
    if d > 0.0 { return 1 } else { return 0 }
}
";
    assert!(diags(src).is_empty(), "{:?}", diags(src));
}

#[test]
fn inference_defaults_and_annotations() {
    let src = "\
fn f() -> i64 {
    let x = 42          // Int var → i64
    let y: i32 = 7      // annotation pins Int var → i32
    let z = x + y       // i64 + i32 → mismatch? no: y is i32, x is var
    z
}
";
    // `x` is an unconstrained int var; `x + y` unifies x with i32; z: i32
    // returned as i64 → E2107 expected.
    assert_has(src, codes::SEM_RETURN_TYPE);
}

// ----- E2xxx diagnostics --------------------------------------------------------

#[test]
fn undeclared_variable() {
    assert_has("fn f() { x + 1 }", codes::SEM_UNDECLARED_VAR);
}

#[test]
fn redefinition() {
    let src = "fn dup() {}\nfn dup() {}\n";
    let d = diags(src);
    assert!(d.iter().any(|d| d.code == Some(codes::SEM_REDEFINITION)));
    // and the label points at the first definition
    let dup = d
        .iter()
        .find(|d| d.code == Some(codes::SEM_REDEFINITION))
        .unwrap();
    assert_eq!(dup.labels.len(), 1);
}

#[test]
fn immutable_assignment() {
    assert_has("fn f() { let x = 1\n x = 2 }", codes::SEM_MUTATE_IMMUTABLE);
}

#[test]
fn mutable_assignment_ok() {
    assert!(diags("fn f() { let mut x = 1\n x = 2 }").is_empty());
}

// ----- type errors --------------------------------------------------------------

#[test]
fn type_mismatch() {
    assert_has("fn f() { let x: i64 = true }", codes::SEM_TYPE_MISMATCH);
    assert_has("fn f() { 1 + true }", codes::SEM_TYPE_MISMATCH);
}

#[test]
fn non_bool_condition() {
    assert_has("fn f() { if 1 { } }", codes::SEM_NOT_BOOL_CONDITION);
}

#[test]
fn arg_count() {
    let src = "fn g(a: i64, b: i64) {}\nfn f() { g(1) }";
    assert_has(src, codes::SEM_ARG_COUNT);
}

#[test]
fn not_callable() {
    assert_has("fn f() { let x = 1\n x() }", codes::SEM_NOT_CALLABLE);
}

#[test]
fn no_such_field() {
    let src = "struct S { a: i64 }\nfn f() { let s = S { a: 1 }\n s.nope }";
    assert_has(src, codes::SEM_NO_FIELD);
}

#[test]
fn missing_field() {
    let src = "struct S { a: i64, b: i64 }\nfn f() { let s = S { a: 1 } }";
    assert_has(src, codes::SEM_MISSING_FIELDS);
}

#[test]
fn unknown_type_in_annotation() {
    assert_has("fn f() { let x: Nope = 1 }", codes::SEM_UNKNOWN_TYPE);
}

#[test]
fn return_type_mismatch() {
    assert_has("fn f() -> i64 { true }", codes::SEM_RETURN_TYPE);
}

#[test]
fn if_value_needs_else() {
    assert_has(
        "fn f() -> i64 { if true { 1 } }",
        codes::SEM_IF_MISSING_ELSE,
    );
}

// ----- enums / match --------------------------------------------------------------

#[test]
fn enum_match_ok() {
    let src = "\
enum Shape { Circle(f64), Empty }
fn f(s: Shape) -> i64 {
    match s { Circle(r) => 1, Empty => 0 }
}
";
    assert!(diags(src).is_empty(), "{:?}", diags(src));
}

#[test]
fn non_exhaustive_match() {
    let src = "\
enum Shape { Circle(f64), Empty }
fn f(s: Shape) -> i64 {
    match s { Circle(r) => 1 }
}
";
    assert_has(src, codes::SEM_NON_EXHAUSTIVE_MATCH);
}

#[test]
fn wildcard_covers_match() {
    let src = "\
enum Shape { Circle(f64), Empty }
fn f(s: Shape) -> i64 {
    match s { Circle(r) => 1, _ => 0 }
}
";
    assert!(diags(src).is_empty(), "{:?}", diags(src));
}

#[test]
fn unknown_variant() {
    let src = "\
enum Shape { Circle(f64) }
fn f(s: Shape) -> i64 {
    match s { Nope(r) => 1, _ => 0 }
}
";
    assert_has(src, codes::SEM_UNKNOWN_VARIANT);
}

// ----- Result / `?` ---------------------------------------------------------------

#[test]
fn result_ok_err_and_match() {
    let src = "\
fn f(ok: bool) -> Result<i64, i64> {
    if ok { Ok(1) } else { Err(2) }
}
fn g(r: Result<i64, i64>) -> i64 {
    match r { Ok(v) => v, Err(e) => e }
}
";
    assert!(diags(src).is_empty(), "{:?}", diags(src));
}

#[test]
fn result_try_propagates() {
    let src = "\
fn inner(ok: bool) -> Result<i64, i64> {
    if ok { Ok(1) } else { Err(2) }
}
fn outer(ok: bool) -> Result<i64, i64> {
    let v = inner(ok)?
    Ok(v + 1)
}
";
    assert!(diags(src).is_empty(), "{:?}", diags(src));
}

#[test]
fn try_on_non_result() {
    let src = "\
fn f() -> i64 {
    let x = 5?
    x
}
";
    assert_has(src, codes::SEM_TYPE_MISMATCH);
}

#[test]
fn try_in_non_result_fn() {
    let src = "\
fn inner() -> Result<i64, i64> { Ok(1) }
fn f() -> i64 {
    let x = inner()?
    x
}
";
    assert_has(src, codes::SEM_TYPE_MISMATCH);
}

#[test]
fn try_error_side_must_match_return() {
    let src = "\
fn inner() -> Result<i64, i64> { Err(1) }
fn outer() -> Result<i64, bool> {
    let x = inner()?
    Ok(x)
}
";
    assert_has(src, codes::SEM_TYPE_MISMATCH);
}

#[test]
fn result_match_non_exhaustive() {
    let src = "\
fn inner() -> Result<i64, i64> { Ok(1) }
fn f() -> i64 {
    match inner() { Ok(v) => v }
}
";
    assert_has(src, codes::SEM_NON_EXHAUSTIVE_MATCH);
}

#[test]
fn result_ctor_arity() {
    let src = "\
fn f() -> Result<i64, i64> { Ok() }
";
    assert_has(src, codes::SEM_ARG_COUNT);
}

/// File-local `Ok`/`Err` names (e.g. an enum's variants) shadow the
/// built-in prelude constructors.
#[test]
fn user_defs_shadow_result_prelude() {
    let src = "\
enum Verdict { Ok(i64), Err(i64) }
fn f() -> i64 {
    let v = Ok(5)
    match v { Ok(x) => x, Err(e) => e }
}
";
    assert!(diags(src).is_empty(), "{:?}", diags(src));
}

#[test]
fn result_nested_and_in_struct() {
    let src = "\
struct Box { r: Result<i64, i64> }
fn f() -> Result<Result<i64, i64>, i64> { Ok(Ok(7)) }
fn g(b: Box) -> i64 {
    match b.r { Ok(v) => v, Err(e) => e }
}
";
    assert!(diags(src).is_empty(), "{:?}", diags(src));
}

// ----- extern fns -----------------------------------------------------------------

#[test]
fn extern_fn_calls() {
    let src = "\
extern \"C\" { fn abs(x: i32) -> i32 }
fn f() -> i64 { abs(-3) }
";
    // abs returns i32; f returns i64 → E2107? No: the return TYPE is i64,
    // call gives i32 → mismatch. Let's check what fires.
    let d = diags(src);
    assert!(
        d.iter().any(|d| d.code == Some(codes::SEM_RETURN_TYPE)),
        "{d:?}"
    );
}

// ----- incrementality --------------------------------------------------------------

/// The marquee test: editing fn `b`'s body must not re-check fn `a`.
///
/// `check_file` calls `typeck_fn(a)` + `typeck_fn(b)`. After the edit,
/// salsa revalidates: `parsed` changed (`no_eq`), `file_items` is equal
/// (signatures untouched → backdated), `fn_body(a)` is equal (a's subtree
/// didn't move — `b` comes later in the file). So `typeck_fn(a)`'s memo is
/// still valid: exactly ONE new `typeck_fn` execution total (for `b`).
#[test]
fn body_edit_preserves_other_fns_typeck() {
    const SRC: &str = "\
fn a() -> i64 { 1 }
fn b() -> i64 { 2 }
";
    let mut db = AuraDatabase::new();
    let file = SourceFile::new(&db, SRC.to_owned(), FileId(0));
    assert!(check_file(&db, file).is_empty());
    db.clear_events();

    // Edit inside b's body only.
    file.set_text(&mut db).to(SRC.replace("{ 2 }", "{ 2 + 3 }"));
    assert!(check_file(&db, file).is_empty());

    assert_eq!(
        db.execution_count("typeck_fn"),
        1,
        "only fn b should re-check; events: {:?}",
        db.events()
    );
}

/// Editing a signature DOES invalidate dependents.
#[test]
fn signature_edit_rechecks() {
    const SRC: &str = "fn a() -> i64 { 1 }\n";
    let mut db = AuraDatabase::new();
    let file = SourceFile::new(&db, SRC.to_owned(), FileId(0));
    assert!(check_file(&db, file).is_empty());
    db.clear_events();

    file.set_text(&mut db).to(SRC.replace("-> i64", "-> f64"));
    // a returns int literal `1` as f64 — Var(Int) can't unify with f64.
    let d = check_file(&db, file).clone();
    assert!(
        d.iter().any(|x| x.code == Some(codes::SEM_RETURN_TYPE)),
        "{d:?}"
    );
    assert!(db.execution_count("typeck_fn") >= 1);
}

// ----- str ---------------------------------------------------------------------

#[test]
fn str_fields_and_equality_clean() {
    let src = "\
fn main() -> i64 {
    let s = \"hello\"
    if s.len == 5 && s == \"hello\" && s != \"\" { 1 } else { 0 }
}
";
    assert!(diags(src).is_empty(), "{:?}", diags(src));
}

#[test]
fn str_param_return_clean() {
    let src = "\
fn id(s: str) -> str { s }
fn main() -> i64 {
    let x: str = \"ok\"
    if id(x) == \"ok\" { 1 } else { 0 }
}
";
    assert!(diags(src).is_empty(), "{:?}", diags(src));
}

#[test]
fn str_unknown_field() {
    assert_has(
        "fn main() -> i64 { let s = \"x\"\n s.foo }",
        codes::SEM_NO_FIELD,
    );
}

#[test]
fn str_order_compare_rejected() {
    assert_has(
        "fn main() -> i64 { if \"a\" < \"b\" { 1 } else { 0 } }",
        codes::SEM_TYPE_MISMATCH,
    );
}

#[test]
fn extern_str_param_diagnosed() {
    assert_has(
        "extern \"C\" { fn take(s: str) -> i64 }\nfn main() -> i64 { take(\"x\") }",
        codes::SEM_EXTERN_AGGREGATE,
    );
}

#[test]
fn extern_struct_param_diagnosed() {
    assert_has(
        "struct P { x: i64 }\nextern \"C\" { fn take(p: P) -> i64 }\nfn main() -> i64 { 0 }",
        codes::SEM_EXTERN_AGGREGATE,
    );
}

#[test]
fn extern_str_return_diagnosed() {
    assert_has(
        "extern \"C\" { fn give() -> str }\nfn main() -> i64 { 0 }",
        codes::SEM_EXTERN_AGGREGATE,
    );
}
