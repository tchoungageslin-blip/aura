# Cheatsheet

The whole language on one page. Every block below is a complete,
compilable program. [Version française](./fr/cheatsheet.md).

## Skeleton

```aura
fn main() -> i64 {
    println("hello")
    0                     // tail expression = return value
}
```

## Variables & types

```aura
fn main() -> i64 {
    let x = 41            // i64, immutable, inferred
    let mut y: i32 = 1    // mutable, annotated
    y = y + 1
    let f: f64 = 1.5
    let b = true          // bool
    let s = "text"        // str
    let v: vec<i64> = vec_new()   // vec<T> needs a concrete T here
    x                     // returns 41
}
```

Types: `i64` `i32` `i16` `i8` `u64` `u32` `u16` `u8` `f64` `f32`
`bool` `str` `vec<T>` `Result<T,E>` `()` and user `struct`/`enum`.
Integers do not auto-widen: `x as i32` converts.

## Control flow

```aura
fn classify(n: i64) -> i64 {
    if n < 0 { -1 } else if n == 0 { 0 } else { 1 }
}

fn main() -> i64 {
    let mut i = 0
    while i < 10 {        // while cond { }
        i = i + 1
        if i == 5 { continue }
        if i == 9 { break }
    }
    loop { break }        // unconditional loop
    classify(i)
}
```

`if` is an expression: `let m = if a > b { a } else { b }`.

## Functions

```aura
fn add(a: i64, b: i64) -> i64 { a + b }   // tail expr, no return

fn add_one(x: i64) -> i64 { x + 1 }

fn main() -> i64 {
    add_one(add(20, 1))                   // calls compose freely
}
```

## Structs

```aura
struct Vec2 { x: f64, y: f64 }

fn main() -> i64 {
    let v = Vec2 { x: 3.0, y: 4.0 }
    println(str_from_f64(v.x))
    if (Vec2 { x: 0.0, y: 0.0 }).x == 0.0 { 0 } else { 1 }
}
```

Struct literals need parens in expression-head position.

## Enums & match

```aura
enum Shape { Circle(f64), Rect(f64, f64), Empty }

fn tag(s: Shape) -> i64 {
    match s {
        Circle(r) => 1,
        Rect(w, h) => 2,
        _ => 0,           // match must be exhaustive
    }
}

fn main() -> i64 { tag(Circle(1.0)) }
```

## Result & ?

```aura
fn parse(s: str) -> Result<i64, str> {
    if s == "x" { Ok(1) } else { Err("bad") }
}

fn main() -> i64 {
    match parse("x") {
        Ok(v) => v,
        Err(e) => { println(e); 0 },
    }
}
```

`expr?` unwraps `Ok` or returns the `Err` early — only inside a
function returning `Result`.

## Strings

```aura
fn main() -> i64 {
    let s = "ab" + "cd"                   // concat
    println(str_from_int(s.len))          // 4 (bytes)
    println(str_from_int(str_get(s, 0)))  // 97 = 'a'
    println(str_slice(s, 1, 3))           // "bc"
    println(str_from_int(42))             // "42"
    0
}
```

`==` compares by content. Ordering (`<`) on `str` is rejected — and
`println` only takes `str`, so wrap numbers in `str_from_int`.

## Vectors

```aura
fn main() -> i64 {
    let v = vec_new()
    vec_push(v, 10)
    vec_push(v, 20)
    vec_set(v, 0, 99)
    println(str_from_int(vec_get(v, 0) + v.len))  // 101
    0
}
```

## I/O, files, args, env

```aura
fn main() -> i64 {
    let a = args()                        // vec<str>, argv[0] included
    println(vec_get(a, 0))
    println(str_from_bool(env("PATH").len > 0))
    println(str_from_bool(read_stdin().len == 0))
    if write_file("out.txt", "hi") {      // -> bool
        match read_file("out.txt") {      // -> Result<str, str>
            Ok(body) => println(body),
            Err(e) => println(e),
        }
    }
    0
}
```

Full list: [Standard Library](./stdlib.md).

## Projects & FFI

`use` + `aura.toml` dependencies for multi-file programs — see
[The Toolchain](./toolchain.md). `extern "C"` declares native
functions (scalar types only):

```aura,ignore
extern "C" { fn GetTickCount64() -> u64 }
```

## Gotchas

- No implicit conversions — `str_from_int(n)` before concatenating,
  and `println` takes `str` only.
- `let` is immutable; `let mut` to reassign.
- Statements end at newline (ASI); a trailing expression returns.
- `match` must be exhaustive — use `_` as the catch-all.
- Compiled bounds/overflow traps exit 101; `aura interp` reports them
  as clean errors instead.
