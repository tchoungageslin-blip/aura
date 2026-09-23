//! Formatter tests — canonical output, idempotence, comment preservation.

use aura_fmt::format_source;

fn fmt(src: &str) -> String {
    format_source(src).expect("source must parse")
}

/// `fmt` is a fixpoint: formatting twice changes nothing.
fn assert_idempotent(src: &str) {
    let once = fmt(src);
    let twice = fmt(&once);
    assert_eq!(
        once, twice,
        "not idempotent:\n--- once ---\n{once}\n--- twice ---\n{twice}"
    );
    // And the formatted output must still parse.
    assert!(
        aura_parser::parse_file(&once, aura_common::FileId(0))
            .diagnostics
            .is_empty(),
        "formatted output doesn't parse:\n{once}"
    );
}

#[test]
fn items_get_canonical_layout() {
    let out = fmt("fn   f(x:i64,y:i64)->i64{x+y}\nstruct P{a:i64,b:i64}\nenum E{A,B(i64)}\n");
    assert_eq!(
        out,
        "fn f(x: i64, y: i64) -> i64 {\n    x + y\n}\n\nstruct P { a: i64, b: i64 }\n\nenum E { A, B(i64) }\n"
    );
    assert_idempotent("fn   f(x:i64,y:i64)->i64{x+y}\nstruct P{a:i64,b:i64}\nenum E{A,B(i64)}\n");
}

#[test]
fn statements_and_exprs_normalize() {
    let src = "fn f() -> i64 {\n    let x=1+2*3\n    let y :i64= x - -1\n    return x\n}\n";
    let out = fmt(src);
    assert_eq!(
        out,
        "fn f() -> i64 {\n    let x = 1 + 2 * 3\n    let y: i64 = x - -1\n    return x\n}\n"
    );
    assert_idempotent(src);
}

#[test]
fn precedence_parens_roundtrip() {
    // `(1 + 2) * 3` must keep its parens; `1 + 2 * 3` must not gain any.
    let src = "fn f() -> i64 { (1 + 2) * 3 + (4 - 5) }\n";
    let out = fmt(src);
    assert!(out.contains("(1 + 2) * 3 + (4 - 5)"), "{out}");
    let src2 = "fn f() -> i64 { 1 + 2 * 3 }\n";
    assert!(fmt(src2).contains("1 + 2 * 3"));
    assert_idempotent(src);
    assert_idempotent(src2);
}

#[test]
fn if_match_and_loops() {
    let src = "\
fn f(n: i64) -> i64 {
    if n > 0 { 1 } else if n < 0 { -1 } else { 0 }
}
fn g(n: i64) -> i64 {
    match n { 0 => 1, _ => 2 }
}
fn h(n: i64) -> i64 {
    let mut i = n
    while i > 0 { i = i - 1 }
    loop { break }
    i
}
";
    assert_idempotent(src);
    let out = fmt(src);
    assert!(
        out.contains("if n > 0 { 1 } else if n < 0 { -1 } else { 0 }"),
        "{out}"
    );
    assert!(out.contains("0 => 1,\n"), "{out}");
    assert!(out.contains("while i > 0 {"), "{out}");
}

#[test]
fn comments_are_preserved() {
    let src = "// header\nfn f() -> i64 {\n    let x = 1 // trailing\n    // before ret\n    return x\n}\n// tail\n";
    let out = fmt(src);
    for c in ["// header", "// trailing", "// before ret", "// tail"] {
        assert!(out.contains(c), "lost {c}: {out}");
    }
    assert_idempotent(src);
}

#[test]
fn block_comments_and_strings() {
    let src =
        "/* multi\n   line */\nfn f() -> i64 {\n    let s = \"a // not a comment\"\n    0\n}\n";
    let out = fmt(src);
    assert!(out.contains("/* multi\n   line */"), "{out}");
    assert!(out.contains("\"a // not a comment\""), "{out}");
    assert_idempotent(src);
}

#[test]
fn wide_signatures_break() {
    let src = "fn wide(a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64, h: i64, i: i64, j: i64, k: i64) -> i64 { a }\n";
    let out = fmt(src);
    assert!(out.contains("(\n"), "expected broken params: {out}");
    assert!(out.contains("    a: i64,\n"), "{out}");
    assert_idempotent(src);
}

#[test]
fn result_and_try_format() {
    let src = "fn f(ok: bool) -> Result<i64, i64> { if ok { Ok(1) } else { Err(2) } }\nfn g() -> Result<i64, i64> { let v = f(true)?\n Ok(v) }\n";
    let out = fmt(src);
    assert!(out.contains("Result<i64, i64>"), "{out}");
    assert!(out.contains("let v = f(true)?"), "{out}");
    assert_idempotent(src);
}

#[test]
fn use_and_extern_items() {
    let src = "use std.io\nextern \"C\" { fn sqrt(x: f64) -> f64 }\n";
    let out = fmt(src);
    assert!(out.contains("use std.io"), "{out}");
    assert!(out.contains("extern \"C\" {"), "{out}");
    assert!(out.contains("fn sqrt(x: f64) -> f64"), "{out}");
    assert_idempotent(src);
}

#[test]
fn struct_lit_and_aggregates() {
    let src = "struct P { x: i64, y: i64 }\nfn f() -> i64 { let p = P{x:1,y:2}\n p.x }\n";
    let out = fmt(src);
    assert!(out.contains("P { x: 1, y: 2 }"), "{out}");
    assert_idempotent(src);
}

#[test]
fn literals_keep_source_spelling() {
    let src = "fn f() -> i64 { let a = 0xFF\n let b = 0b1010\n a }\n";
    let out = fmt(src);
    assert!(out.contains("0xFF"), "{out}");
    assert!(out.contains("0b1010"), "{out}");
    assert_idempotent(src);
}

#[test]
fn broken_source_refuses_to_format() {
    assert!(format_source("fn f( -> {}").is_err());
    assert!(format_source("} stray").is_err());
}

#[test]
fn blank_lines_between_stmts_preserved() {
    let src = "fn f() -> i64 {\n    let a = 1\n\n    let b = 2\n    a + b\n}\n";
    let out = fmt(src);
    assert!(out.contains("let a = 1\n\n    let b = 2"), "{out}");
    assert_idempotent(src);
}
