# Error Index

Every diagnostic Aura emits carries a stable `E` code. Look one up with
`aura doc E2101` or find it here. Each entry shows a minimal program
that triggers the error and how to fix it.

## Lexer (E0001–E0006)

### E0001 — invalid character
A byte the lexer cannot tokenize (e.g. `` ` `` or an unprintable
control character) appeared in the source.

```aura,fail
fn main() -> i64 { let x = 1 ` 2 }
```

**Fix:** remove or replace the stray character.

### E0002 — unterminated string
A `"` literal runs past the end of its line/file without closing.

```aura,fail
fn main() -> i64 { println("oops)
```

**Fix:** close the string: `println("oops")`.

### E0003 — unterminated block comment
A `/*` comment is missing its closing `*/`.

```aura,fail
fn main() -> i64 { /* never closed
```

**Fix:** terminate the comment with `*/`. Block comments nest.

### E0004 — invalid number
A numeric literal is malformed or out of range (`u128` max for
integers).

```aura,fail
fn main() -> i64 { 340282366920938463463374607431768211456999 }
```

**Fix:** shrink the literal or store the value differently (e.g. as a
`str` and parse it).

### E0005 — invalid escape
A string escape isn't one of `\n \t \r \\ \" \' \0`.

```aura,fail
fn main() -> i64 { println("bad\q")
```

**Fix:** use a valid escape or escape the backslash: `"bad\\q"`.

### E0006 — invalid UTF-8
The source file isn't valid UTF-8. Save the file as UTF-8.

## Parser (E1001–E1008)

### E1001 — unexpected token
A token appeared where the grammar doesn't allow it. The message names
what was expected.

### E1002 — expected expression
A statement or item showed up where an expression was required — most
often `return`/`break` as a `match` arm body or in tail position.

```aura,fail
fn f(b: bool) -> Result<i64, str> {
    match b { Ok(v) => v, Err(e) => return 1 }
}
```

**Fix:** use a value (`exit(1)`, a block, `!`-typed exprs work) or a
block arm: `Err(e) => { exit(1) }`.

### E1003 — expected identifier
A name was required (after `fn`, `let`, `struct`, field access, …).

### E1004 — expected type
A type annotation was required (after `:` in signatures, `->`, etc.).

### E1005 — unclosed delimiter
An opening `(`, `[` or `{` never got its closer.

### E1006 — expected item
Something that isn't an item (`fn`, `struct`, `enum`, `extern`, `use`)
appeared at file top level.

### E1007 — maximum parse depth
Expression nesting exceeded the 256-level cap — usually runaway left
brackets. Split the expression into `let` bindings.

### E1008 — binary operator after a block expression
`if`/`while`/blocks can't feed a binary operator unparenthesized —
`if c { 1 } + 2` would be ambiguous.

```aura,fail
fn f(c: bool) -> i64 { if c { 1 } + 2 else { 3 } }
```

**Fix:** parenthesize: `(if c { 1 } else { 2 }) + 3`.

## Semantic (E2001–E2112)

### E2001 — undeclared variable
A name is used that no `let`, `fn`, `struct`, `enum` or parameter
introduces.

```aura,fail
fn main() -> i64 { missing + 1 }
```

**Fix:** declare it (`let missing = ...`) or check the spelling.

### E2002 — use before initialization
A `let mut` binding is read before its first assignment on some path.

### E2003 — redefinition
Two items or bindings share a name in the same scope. Aura has no
shadowing for items; locals may shadow only in nested scopes.

### E2004 — cannot mutate immutable
Assignment to a `let` binding declared without `mut`, or to a field of
an immutable value.

```aura,fail
fn main() -> i64 {
    let x = 1
    x = 2
}
```

**Fix:** declare it `let mut x = 1`.

### E2010 — circular dependency
Module/`use` cycle or a function/type that recursively requires itself
to be resolved first.

### E2100 — type mismatch
The inferred type doesn't match the expected one.

```aura,fail
fn f() -> i64 { "hello" }
```

**Fix:** make the types agree; `str` doesn't auto-convert to numbers —
use `str_from_int`/`f64_from_int` the other way.

### E2101 — `if` used as a value requires `else`
An `if` in expression position whose branches produce a value must be
exhaustive. Statement-position `if`s with `()`/`!` bodies don't need
`else`.

```aura,fail
fn f(x: i64) -> i64 { if x > 0 { 1 } }
```

**Fix:** add `else { ... }`, or make the branch statements (`x = 1`).

### E2102 — wrong argument count
A call passes the wrong number of arguments for the callee's
signature.

### E2103 — not callable
`expr(...)` where `expr` isn't a function.

### E2104 — no such field
`x.field` where the struct/enum/`str`/`vec` has no field of that name.
`str` has `len`/`ptr`; `vec` has `len`/`cap`/`ptr`.

### E2105 — condition is not `bool`
`if`/`while` conditions must be `bool` — integers don't coerce.

```aura,fail
fn f(x: i64) -> i64 { if x { 1 } else { 0 } }
```

**Fix:** compare explicitly: `if x != 0`.

### E2106 — non-exhaustive match
`match` doesn't cover every variant/value — add the missing arms or a
`_` wildcard.

### E2107 — return type mismatch
A `return` expression's type differs from the function's `->` type.

### E2108 — unknown type
A type annotation names a type that doesn't exist (check `i64`,
`str`, `vec<T>`, `Result<T,E>`…).

### E2109 — unknown variant
A `match` pattern or constructor names an enum variant that isn't
declared. Variants are flat names — `Circle`, not `Shape::Circle`.

### E2110 — missing struct fields
A struct literal omits required fields.

```aura,fail
struct P { x: i64, y: i64 }
fn f() -> i64 {
    let p = P { x: 1 }
    0
}
```

**Fix:** supply every field: `P { x: 1, y: 0 }`.

### E2111 — cannot infer type
The checker can't pin down an inference variable — typical cause:
`vec_new()` with no later element use.

**Fix:** annotate: `let v: vec<i64> = vec_new()`.

### E2112 — extern signatures may only use scalars
`extern "C"` declarations can't pass `str`, `vec`, structs or enums —
only numbers, `bool`, pointers, `()`.

## Codegen / linking (E3001–E3005)

### E3001 — internal codegen error
Compiler bug — please report it with the source that triggers it.

### E3002 — linker failed
`lld-link` ran but rejected the objects; the stderr output follows.

### E3003 — linker not found
No usable linker: the distribution ships `lld-link.exe` beside
`aura.exe` — reinstall or restore it. On a dev machine, any Rust
toolchain provides one; `AURA_LINKER` can point at a driver.

### E3004 — unsupported type for codegen
A construct reached codegen that the backend can't emit yet.

### E3005 — invalid `main` signature
`main` must be `fn main() -> i64` (no parameters).

```aura,fail
fn main() { 0 }
```

**Fix:** declare `fn main() -> i64 { ... }`.
