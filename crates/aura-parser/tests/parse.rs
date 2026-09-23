use aura_common::FileId;
use aura_parser::parse_file;

fn dump(src: &str) -> String {
    parse_file(src, FileId(0)).dump()
}

fn diags(src: &str) -> Vec<String> {
    parse_file(src, FileId(0))
        .diagnostics
        .iter()
        .map(|d| format!("{}: {}", d.code.unwrap_or("-----"), d.message))
        .collect()
}

// ---------- expressions ----------

#[test]
fn precedence() {
    insta::assert_snapshot!(dump("fn f() { 1 + 2 * 3 }"), @"(fn f (params) (block (+ 1 (* 2 3))))");
    insta::assert_snapshot!(dump("fn f() { a * b + c }"), @"(fn f (params) (block (+ (* a b) c)))");
    insta::assert_snapshot!(dump("fn f() { a == b && c || d }"), @"(fn f (params) (block (|| (&& (== a b) c) d)))");
    insta::assert_snapshot!(dump("fn f() { -x * !y }"), @"(fn f (params) (block (* (neg x) (not y))))");
    insta::assert_snapshot!(dump("fn f() { a < b == c > d }"), @"(fn f (params) (block (== (< a b) (> c d))))");
}

#[test]
fn associativity() {
    insta::assert_snapshot!(dump("fn f() { 1 - 2 - 3 }"), @"(fn f (params) (block (- (- 1 2) 3)))");
    insta::assert_snapshot!(dump("fn f() { a = b = c }"), @"(fn f (params) (block (= a (= b c))))");
}

#[test]
fn postfix() {
    insta::assert_snapshot!(dump("fn f() { a.b.c }"), @"(fn f (params) (block (. (. a b) c)))");
    insta::assert_snapshot!(dump("fn f() { f(x)(y) }"), @"(fn f (params) (block (call (call f x) y)))");
    insta::assert_snapshot!(dump("fn f() { a.b(c).d }"), @"(fn f (params) (block (. (call (. a b) c) d)))");
    insta::assert_snapshot!(dump("fn f() { maybe()? }"), @"(fn f (params) (block (try (call maybe))))");
}

#[test]
fn literals() {
    insta::assert_snapshot!(dump("fn f() { 0xFF + 0b11 + 1_000 }"), @"(fn f (params) (block (+ (+ 255 3) 1000)))");
    insta::assert_snapshot!(dump(r#"fn f() { "hi\n" }"#), @"(fn f (params) (block \"hi\\n\"))");
    insta::assert_snapshot!(dump("fn f() { true && false }"), @"(fn f (params) (block (&& true false)))");
    insta::assert_snapshot!(dump("fn f() { () }"), @"(fn f (params) (block ()))");
}

// ---------- statements ----------

#[test]
fn let_decls() {
    insta::assert_snapshot!(dump("fn f() { let x = 1 }"), @"(fn f (params) (block (let x = 1)))");
    insta::assert_snapshot!(dump("fn f() { let mut y: f64 = 2.0 }"), @"(fn f (params) (block (let-mut y: f64 = 2)))");
}

#[test]
fn control_flow() {
    insta::assert_snapshot!(
        dump("fn f() { if a { x } else if b { y } else { z } }"),
        @"(fn f (params) (block (if a (block x) (if b (block y) (block z)))))"
    );
    insta::assert_snapshot!(
        dump("fn f() { while x { work() } }"),
        @"(fn f (params) (block (while x (block (call work)))))"
    );
    insta::assert_snapshot!(
        dump("fn f() { loop { break } }"),
        @"(fn f (params) (block (loop (block break))))"
    );
    insta::assert_snapshot!(dump("fn f() { return 42 }"), @"(fn f (params) (block (return 42)))");
    insta::assert_snapshot!(dump("fn f() { return\n42 }"), @"(fn f (params) (block (return) 42))");
}

#[test]
fn if_needs_parens_for_binary() {
    // `if c {1} + 2` → block-like lhs + binary op → error E1008
    let d = diags("fn f() { let x = if c { 1 } + 2 }");
    assert!(d.iter().any(|m| m.contains("E1008")), "{d:?}");
    // parenthesized is fine
    insta::assert_snapshot!(
        dump("fn f() { let x = (if c { 1 } else { 2 }) + 3 }"),
        @"(fn f (params) (block (let x = (+ (paren (if c (block 1) (block 2))) 3))))"
    );
}

#[test]
fn no_struct_literal_in_if_head() {
    // `{ a: 1 }` after `== Point` must be the if-body, not a struct literal
    let d = dump("fn f() { if x == Point { a } }");
    insta::assert_snapshot!(d, @"(fn f (params) (block (if (== x Point) (block a))))");
    // but struct literal works in normal position
    insta::assert_snapshot!(
        dump("fn f() { let p = Point { x: 1.0, y: 2.0 } }"),
        @"(fn f (params) (block (let p = (Point x: 1 y: 2))))"
    );
}

#[test]
fn match_expr() {
    insta::assert_snapshot!(
        dump("fn f() { match s { Circle(r) => r, Rect(w, h) => w * h, _ => 0 } }"),
        @"(fn f (params) (block (match s (arm Circle(r) => r) (arm Rect(w, h) => (* w h)) (arm _ => 0))))"
    );
}

#[test]
fn unsafe_block() {
    insta::assert_snapshot!(
        dump("fn f() { unsafe { raw() } }"),
        @"(fn f (params) (block (unsafe (block (call raw)))))"
    );
}

// ---------- items ----------

#[test]
fn items() {
    insta::assert_snapshot!(
        dump("struct Point { x: f64, y: f64 }"),
        @"(struct Point (x: f64) (y: f64))"
    );
    insta::assert_snapshot!(
        dump("enum Shape { Circle(f64) Rect(f64, f64) Empty }"),
        @"(enum Shape Circle(f64) Rect(f64, f64) Empty)"
    );
    insta::assert_snapshot!(dump("use std.io"), @"(use std io)");
    insta::assert_snapshot!(
        dump("fn add(a: i32, b: i32) -> i32 { a + b }"),
        @"(fn add (params a: i32 b: i32) -> i32 (block (+ a b)))"
    );
    insta::assert_snapshot!(
        dump("extern \"C\" { fn puts(s: *const u8) -> i32 }"),
        @"(extern C (fn puts (params s: *const u8) -> i32 <bodiless>))"
    );
}

#[test]
fn block_tail_vs_stmt() {
    // trailing expr without `;` is the tail (block value)
    insta::assert_snapshot!(
        dump("fn f() { let x = 1\nx + 1 }"),
        @"(fn f (params) (block (let x = 1) (+ x 1)))"
    );
    // `;` makes it a statement
    insta::assert_snapshot!(
        dump("fn f() { x + 1; 0 }"),
        @"(fn f (params) (block (+ x 1) 0))"
    );
}

// ---------- ASI ----------

#[test]
fn asi_newline_separates() {
    insta::assert_snapshot!(
        dump("fn f() { a\nb }"),
        @"(fn f (params) (block a b))"
    );
}

#[test]
fn asi_continuation_across_newline() {
    // `.foo` after newline continues the expression (method-chain style)
    insta::assert_snapshot!(
        dump("fn f() { a\n.b() }"),
        @"(fn f (params) (block (call (. a b))))"
    );
    // binary op before newline continues too
    insta::assert_snapshot!(
        dump("fn f() { let x = 1 +\n2 }"),
        @"(fn f (params) (block (let x = (+ 1 2))))"
    );
}

#[test]
fn semicolons_optional_everywhere() {
    let src = "fn f() { let a = 1; let b = 2\na + b }";
    insta::assert_snapshot!(dump(src), @"(fn f (params) (block (let a = 1) (let b = 2) (+ a b)))");
}

// ---------- error recovery ----------

#[test]
fn recovers_multiple_errors() {
    let d = diags("fn broken( {\nlet x = = 2\nlet y = 3\n}\nfn ok() { 1 }");
    // must have produced diagnostics AND still parsed `fn ok`
    assert!(!d.is_empty());
    let f = parse_file(
        "fn broken( {\nlet x = = 2\nlet y = 3\n}\nfn ok() { 1 }",
        FileId(0),
    );
    assert!(!f.items.is_empty());
}

#[test]
fn unclosed_delimiters() {
    assert!(!diags("fn f() { (1 + 2").is_empty());
    assert!(!diags("fn f() { f(1, 2").is_empty());
    assert!(!diags("fn f() { let x = Point { a: 1").is_empty());
}

#[test]
fn deep_nesting_no_overflow() {
    // 300 nested parens > MAX_DEPTH → graceful error, not stack overflow
    let src = format!("fn f() {{ {}1{} }}", "(".repeat(300), ")".repeat(300));
    let f = parse_file(&src, FileId(0));
    assert!(f.diagnostics.iter().any(|d| d.code == Some("E1007")));
}

#[test]
fn long_left_assoc_chain_no_overflow() {
    // 10_000 `+`s — iterative loop handles this; recursion would overflow
    let expr = std::iter::repeat_n("1", 10_000)
        .collect::<Vec<_>>()
        .join(" + ");
    let src = format!("fn f() {{ {expr} }}");
    let f = parse_file(&src, FileId(0));
    assert!(f.diagnostics.is_empty(), "{:?}", f.diagnostics);
}

#[test]
fn let_requires_init() {
    let d = diags("fn f() { let x }");
    assert!(!d.is_empty());
}

#[test]
fn snapshot_full_program() {
    let src = r#"
struct Vec2 { x: f64, y: f64 }

fn length(v: Vec2) -> f64 {
    let sq = v.x * v.x + v.y * v.y
    sqrt(sq)
}

fn main() {
    let p = Vec2 { x: 3.0, y: 4.0 }
    let len = length(p)
    if len > 0.0 {
        print_f64(len)
    } else {
        print_str("zero")
    }
}
"#;
    insta::assert_snapshot!(dump(src));
}
