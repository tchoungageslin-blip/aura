//! `aura-interp` tests — the reference semantics every backend is measured
//! against.

use aura_interp::{InterpError, RunError, run_source};

fn run(src: &str) -> i64 {
    run_source(src).expect("interp")
}

fn run_err(src: &str) -> InterpError {
    match run_source(src) {
        Err(RunError::Interp(e)) => e,
        other => panic!("expected interp error, got {other:?}"),
    }
}

#[test]
fn arithmetic_and_return() {
    assert_eq!(run("fn main() -> i64 { 6 * 7 }"), 42);
    assert_eq!(run("fn main() -> i64 { (1 + 2) * 3 - 4 }"), 5);
    assert_eq!(run("fn main() -> i64 { 10 % 3 }"), 1);
}

#[test]
fn locals_and_assign() {
    let src = "fn main() -> i64 {\n    let mut x = 1\n    x = x + 41\n    x\n}";
    assert_eq!(run(src), 42);
}

#[test]
fn fn_calls_and_params() {
    let src = "fn add(a: i64, b: i64) -> i64 { a + b }\nfn main() -> i64 { add(40, 2) }";
    assert_eq!(run(src), 42);
}

#[test]
fn if_else() {
    let src = "fn f(x: i64) -> i64 { if x > 0 { 1 } else { 2 } }\nfn main() -> i64 { f(9) }";
    assert_eq!(run(src), 1);
}

#[test]
fn while_loop() {
    let src = "fn main() -> i64 {\n    let mut i = 0\n    while i < 10 { i = i + 3 }\n    i\n}";
    assert_eq!(run(src), 12);
}

#[test]
fn loop_break_continue() {
    let src = "fn main() -> i64 {
        let mut i = 0
        let mut acc = 0
        loop {
            i = i + 1
            if i == 3 { continue }
            if i > 6 { break }
            acc = acc + i
        }
        acc
    }";
    assert_eq!(run(src), 1 + 2 + 4 + 5 + 6);
}

#[test]
fn return_inside_nested_block() {
    // `return` inside an `if` block exits the fn — not the block.
    let src = "fn f() -> i64 { if true { return 7 } 0 }\nfn main() -> i64 { f() }";
    assert_eq!(run(src), 7);
}

#[test]
fn break_inside_nested_block() {
    let src = "fn main() -> i64 {
        let mut i = 0
        while true {
            if i == 5 { break }
            i = i + 1
        }
        i
    }";
    assert_eq!(run(src), 5);
}

#[test]
fn structs_and_fields() {
    let src = "struct Point { x: i64, y: i64 }
fn main() -> i64 {
    let p = Point { x: 3, y: 4 }
    p.x * p.x + p.y * p.y
}";
    assert_eq!(run(src), 25);
}

#[test]
fn enum_match() {
    let src = "enum Shape { Circle(i64), Rect(i64, i64), Empty }
fn area(s: Shape) -> i64 {
    match s {
        Circle(r) => r * r,
        Rect(w, h) => w * h,
        Empty => 0,
    }
}
fn main() -> i64 { area(Rect(3, 4)) }";
    assert_eq!(run(src), 12);
}

#[test]
fn match_wildcard_and_literal() {
    let src = "fn f(x: i64) -> i64 { match x { 42 => 1, _ => 0 } }
fn main() -> i64 { f(42) }";
    assert_eq!(run(src), 1);
}

#[test]
fn result_ok_and_try() {
    let src = "fn inner(ok: bool) -> Result<i64, i64> { if ok { Ok(10) } else { Err(3) } }
fn outer() -> Result<i64, i64> {
    let v = inner(true)?
    Ok(v + 1)
}
fn main() -> i64 {
    match outer() { Ok(v) => v, Err(e) => e }
}";
    assert_eq!(run(src), 11);
}

#[test]
fn result_err_propagates() {
    let src = "fn inner(ok: bool) -> Result<i64, i64> { if ok { Ok(10) } else { Err(3) } }
fn outer() -> Result<i64, i64> {
    let v = inner(false)?
    Ok(v + 1)
}
fn main() -> i64 {
    match outer() { Ok(v) => v, Err(e) => e }
}";
    assert_eq!(run(src), 3);
}

#[test]
fn extern_sqrt_builtin() {
    let src = "extern \"C\" { fn sqrt(x: f64) -> f64; }
fn main() -> i64 { if sqrt(9.0) == 3.0 { 8 } else { 0 } }";
    assert_eq!(run(src), 8);
}

#[test]
fn unsafe_block() {
    let src = "fn main() -> i64 { unsafe { 42 } }";
    assert_eq!(run(src), 42);
}

#[test]
fn div_by_zero_is_a_trap() {
    assert_eq!(
        run_err("fn main() -> i64 { 1 / 0 }"),
        InterpError::DivByZero
    );
}

#[test]
fn missing_main() {
    assert_eq!(run_err("fn f() -> i64 { 0 }"), InterpError::NoMain);
}

#[test]
fn diagnostics_on_bad_source() {
    let r = run_source("fn main() -> i64 {");
    assert!(matches!(r, Err(RunError::Diagnostics(_))));
}

#[test]
fn str_len_and_equality() {
    assert_eq!(
        run(
            "fn main() -> i64 { let s = \"hello\"\n if s.len == 5 && s == \"hello\" { 7 } else { 0 } }"
        ),
        7
    );
}

#[test]
fn str_param_and_return() {
    assert_eq!(
        run(
            "fn id(s: str) -> str { s }\nfn main() -> i64 { if id(\"ok\") == \"ok\" { 3 } else { 0 } }"
        ),
        3
    );
}

#[test]
fn str_field_in_struct() {
    assert_eq!(
        run(
            "struct P { s: str }\nfn main() -> i64 { let p = P { s: \"ab\" }\n if p.s == \"ab\" { 4 } else { 0 } }"
        ),
        4
    );
}

#[test]
fn str_match_pattern() {
    assert_eq!(
        run("fn main() -> i64 { match \"hi\" { \"hi\" => 1, _ => 0 } }"),
        1
    );
}

#[test]
fn str_ptr_is_opaque_int() {
    assert_eq!(
        run("fn main() -> i64 { let s = \"x\"\n if s.ptr == s.ptr { 2 } else { 0 } }"),
        2
    );
}

#[test]
fn builtin_exit_terminates_program() {
    assert_eq!(
        run("fn f() -> i64 { exit(5) }\nfn main() -> i64 { f() }"),
        5
    );
}

#[test]
fn builtin_println_returns_unit() {
    // println writes to the test process's stdout — harmless output,
    // and the call must produce `()` so the fn tail stays i64.
    assert_eq!(run("fn main() -> i64 { println(\"interp\")\n 9 }"), 9);
}

#[test]
fn str_add_concat() {
    let src = "fn main() -> i64 { let s = \"foo\" + \"bar\" + \"\"\n if s == \"foobar\" && s.len == 6 { 3 } else { 0 } }";
    assert_eq!(run(src), 3);
}
