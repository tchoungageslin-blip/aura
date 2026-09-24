# The Language

A `.aura` file is a sequence of item declarations. Statements are
separated by newlines (ASI) — the last expression of a block is its
value; no `;` or `return` is needed for tail returns.

## Lexical structure

```aura
// line comment
/* block comment — may not nest */

let million = 1_000_000        // `_` separators anywhere in digits
let hex = 0xFF                 // hex
let bin = 0b1010               // binary
let oct = 0o17                 // octal
let big = 1.5e3                // float exponent
let s = "a\nb\t\"q\"\\"        // escapes: \n \t \r \0 \\ \"
```

Keywords: `fn` `let` `mut` `if` `else` `while` `loop` `return`
`struct` `enum` `match` `use` `unsafe` `extern` `break` `continue`
`true` `false`. `final` is deliberately **not** a keyword.

## Types

| Category | Types |
|----------|-------|
| Signed integers | `i8` `i16` `i32` `i64` `i128` `isize` |
| Unsigned integers | `u8` `u16` `u32` `u64` `u128` `usize` |
| Floats | `f32` `f64` |
| Other primitives | `bool`, `str` |
| Generic builtins | `vec<T>`, `Result<T, E>` |
| User types | `struct`, `enum` |
| Other | `*const T`, `*mut T`, tuples `(A, B)`, `()` (unit), `!` (never) |

Untyped integer literals unify with any integer type; `f64` is the
default float. `!` is a subtype of everything — `exit(1)` type-checks
anywhere a value is expected.

## Functions

```aura
fn add(a: i64, b: i64) -> i64 { a + b }

fn main() -> i64 {
    add(1, 2)
}
```

`main` must be `fn main() -> i64` — its value becomes the process exit
code.

## Bindings

```aura
let x = 1          // inferred
let y: i32 = 7     // annotated
let mut z = 0      // mutable
z = z + 1
```

## Control flow

`if` is an expression; used as a value it **requires `else`**:

```aura
let x = -5
let sign = if x < 0 { -1 } else { 1 }
if sign < 0 { return 1 }    // statement position — else optional
```

Loops: `while cond { }`, `loop { }`, `break`, `continue`.
There is no `for` — `while` covers it.

## Operators

Highest precedence first. All binary operators are left-associative
except `=`, which is right-associative.

| Level | Operators |
|-------|-----------|
| postfix | `f(args)`, `x.field`, `expr?` |
| unary | `-x` (negate), `!b` (not) |
| 13 | `*` `/` `%` |
| 11 | `+` `-` (also `str + str` concat) |
| 9 | `<` `<=` `>` `>=` |
| 7 | `==` `!=` |
| 5 | `&&` (short-circuit) |
| 3 | `||` (short-circuit) |
| 1 | `=` (assignment — statement, not an expression value) |

Arithmetic on `vec`/`str`/`struct` operands is rejected; `str` supports
`+` and `==`/`!=` only. Integer division or `%` by zero aborts the
process — `aura interp` reports a clean "division by zero" while
compiled code takes a hardware trap; bounds checks on
`vec_get`/`str_get` exit 101 in both modes.

## Structs

```aura
struct Vec2 { x: f64, y: f64 }

fn len2(v: Vec2) -> f64 { v.x * v.x + v.y * v.y }
```

```aura
struct Vec2 { x: f64, y: f64 }

fn main() -> i64 {
    let v = Vec2 { x: 3.0, y: 4.0 }
    let n = v.x
    0
}
```

Struct literals are banned in expression-head position — parenthesize
or bind first:

```aura
struct Vec2 { x: f64, y: f64 }

fn main() -> i64 {
    // if Vec2{x:0.0,y:0.0}.x == 0.0 { ... }   // error
    if (Vec2 { x: 0.0, y: 0.0 }).x == 0.0 { 0 } else { 1 }
}
```

## Enums and match

```aura
enum Shape { Circle(f64), Rect(f64, f64), Empty }

fn tag(s: Shape) -> i64 {
    match s {
        Circle(r) => 1,
        Rect(w, h) => 2,
        Empty => 0,
    }
}
```

`match` must be exhaustive. Patterns include literals (integers,
`str`, `true`/`false`), enum variants with bindings, `_` wildcards,
and `Ok`/`Err`.

## Result and `?`

`Result<T, E>` is built in. `expr?` unwraps `Ok` or returns the `Err`
early — the function's return type must be a `Result` with a matching
error type:

```aura
fn inner(ok: bool) -> Result<i64, str> {
    if ok { Ok(10) } else { Err("nope") }
}

fn outer(ok: bool) -> Result<i64, str> {
    let v = inner(ok)? + 1
    Ok(v)
}
```

## Strings and vectors

`str` is a fat `{ptr, len}` value. `.len` is the byte length, `.ptr`
the data pointer; `+` concatenates (heap-allocates); `==` compares by
content. Ordering comparisons on `str` are rejected.

`vec<T>` is a growable buffer (`{ptr, len, cap}`) manipulated through
builtins — see [Standard Library](./stdlib.md). `v.len` and `v.cap`
are fields; `vec == vec` is rejected.

## Extern FFI and `unsafe`

```aura
extern "C" { fn sqrt(x: f64) -> f64 }
```

Extern signatures may only use scalar types — aggregates (`str`,
`vec`, structs, enums) are rejected with `E2112`. (`sqrt` is also a
prelude builtin, so this decl is unnecessary.)

`unsafe { ... }` is a block expression. In the current alpha it marks
intent only — ARC operations are not injected inside unsafe blocks, so
they are the escape hatch planned for raw-pointer work (`*const T`,
`*mut T` types exist but no dereference syntax yet).

## Projects

`use` declarations plus `aura.toml` dependencies give multi-file
programs — see [The Toolchain](./toolchain.md).

## Grammar (reference)

EBNF — `?` optional, `*` zero or more, `|` alternative, `NL` newline.
The parser is a Pratt expression parser; `infix-op` binding powers are
in the operator table above.

```text
file        = item*
item        = fn | struct | enum | use | extern-block
fn          = "fn" ident "(" params? ")" ("->" type)? block
params      = param ("," param)*            param = ident ":" type
struct      = "struct" ident "{" field ("," field)* "}"
field       = ident ":" type
enum        = "enum" ident "{" variant ("," variant)* "}"
variant     = ident ("(" type ("," type)* ")")?
use         = "use" path ";"?
extern-block= "extern" strlit "{" fn-sig* "}"
fn-sig      = "fn" ident "(" params? ")" ("->" type)?

type        = ident ("<" type ("," type)* ">")?
            | "*" ("const"|"mut") type
            | "(" type ("," type)* ")"

block       = "{" stmt* "}"
stmt        = let | expr NL
let         = "let" "mut"? ident (":" type)? "=" expr

expr        = literal | ident | block | if | match | unsafe | paren
            | unop expr | expr infix expr | call | field | expr "?"
literal     = int | float | strlit | "true" | "false" | "(" ")"
if          = "if" expr block ("else" (if | block))?
match       = "match" expr "{" arm* "}"
arm         = pattern "=>" expr ","
pattern     = "_" | literal | ident | ident "(" pattern ("," pattern)* ")"
call        = expr "(" expr ("," expr)* ")"
field       = expr "." ident
unsafe      = "unsafe" block
loop        = "while" expr block | "loop" block
ctrl        = "return" expr? | "break" | "continue"
```

Notes:

- **ASI**: a newline ends a statement unless the line ends inside
  `()`, `{}`, `[]` or after a binary operator/comma — the parser
  continues the expression.
- Struct literals are not allowed in expression-head position
  (`if S{..}.f == ...`) — parenthesize first.
- `=` parses as an expression syntactically but the type checker
  requires it in statement position.
