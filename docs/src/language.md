# The Language

A `.aura` file is a sequence of item declarations. Statements are
separated by newlines (ASI) — the last expression of a block is its
value; no `;` or `return` is needed for tail returns.

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
let sign = if x < 0 { -1 } else { 1 }
if cond { return 1 }        // statement position — else optional
```

Loops: `while cond { }`, `loop { }`, `break`, `continue`.

## Structs

```aura
struct Vec2 { x: f64, y: f64 }
let v = Vec2 { x: 3.0, y: 4.0 }
let n = v.x
```

Struct literals are banned in expression-head position — parenthesize
or bind first:

```aura
// if Vec2{x:0.0,y:0.0}.x == 0.0 { ... }   // error
if (Vec2 { x: 0.0, y: 0.0 }).x == 0.0 { 0 } else { 1 }
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

## Extern FFI

```aura
extern "C" { fn sqrt(x: f64) -> f64 }
```

Extern signatures may only use scalar types — aggregates (`str`,
`vec`, structs, enums) are rejected with `E2112`. (`sqrt` is also a
prelude builtin, so this decl is unnecessary.)

## Projects

`use` declarations plus `aura.toml` dependencies give multi-file
programs — see [The Toolchain](./toolchain.md).

## Reserved identifiers

`final` is deliberately **not** a keyword — it remains a usable name.
