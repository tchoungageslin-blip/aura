# Effective Aura

Idioms, anti-patterns, and worked recipes. Every example here is
checked against the compiler by the docs gate.

## Idioms

### Let the tail expression return

```aura
fn abs(x: i64) -> i64 {
    if x < 0 { -x } else { x }    // not: if ... { return -x }
}
```

`return` is for *early* exits. The last expression is the value —
writing `return` at the end adds noise.

### Propagate with `?`, handle with `match`

```aura
fn load(path: str) -> Result<str, str> {
    let text = read_file(path)?      // early-return on Err
    Ok(text + "\n")
}

fn main() -> i64 {
    match load("data.txt") {
        Ok(t) => {
            println(t)
            0
        }
        Err(e) => {
            eprintln("failed: " + e)
            1
        }
    }
}
```

Rule of thumb: library-ish functions return `Result`; `main` (and
user-facing edges) `match`es on it and picks an exit code.

### Build strings by concatenation, collect into `vec`

```aura
fn hexes(bytes: vec<i64>) -> str {
    let mut out = ""
    let mut i = 0
    while i < bytes.len {
        out = out + str_from_byte(vec_get(bytes, i))
        i = i + 1
    }
    out
}
```

There is no string builder or `join` yet — `+` on `str` heap-allocates
and is the idiom. For byte-level work, keep data in `vec<i64>` and
convert at the end.

### `usize` for sizes, `i64` for values

`v.len`/`v.cap`/`s.len` are `usize`; counters you print are easiest as
`i64` (`str_from_int` accepts any int width). Mixing widths in one
expression is an error — keep a single width per computation.

### Name errors with enum variants when `str` isn't enough

```aura
enum Io { NotFound, Denied, Other(str) }

fn classify(path: str) -> Result<str, Io> {
    if env("SIMULATE_FAIL") == "1" { Err(Denied) } else {
        match read_file(path) {
            Ok(t) => Ok(t),
            Err(e) => Err(Other(e)),
        }
    }
}
```

`Result<T, str>` is fine for scripts; an `enum` error payload scales
when callers need to distinguish cases.

## Anti-patterns

- **`let mut` everywhere.** Mutability is for reassignment — loop
  counters, accumulators, `vec`s you resize. A `let` you never reassign
  should not be `mut` (E2004 catches assignments, but declaring `mut`
  anyway hides intent).
- **Fighting expression orientation.** `if`/`match`/blocks are
  expressions — prefer `let x = if c { a } else { b }` over
  declare-then-assign in both arms.
- **Catching errors too early.** `match`ing a `Result` only to
  `eprintln` + `exit(1)` in a helper duplicates `main`'s job. Return
  the `Result` and let the edge decide.
- **Str slicing without thinking about bytes.** `str_slice`/`str_get`
  are *byte* operations. ASCII is safe; UTF-8 multi-byte content can be
  cut mid-sequence (the result is still bytes, but printing it may
  look wrong).
- **`vec == vec`.** Rejected by design — compare `.len` then elements,
  or maintain a count.

## Cookbook

### Echo the arguments

```aura
fn main() -> i64 {
    let argv = args()                    // vec<str>, program name first
    let mut i = 1
    while i < argv.len {
        println(vec_get(argv, i))
        i = i + 1
    }
    0
}
```

### Read a config value with a default

```aura
fn config(name: str, default: str) -> str {
    let v = env(name)
    if v.len == 0 { default } else { v }
}

fn main() -> i64 {
    println("threads=" + config("THREADS", "4"))
    0
}
```

`env` returns `""` when unset — empty string doubles as "absent".

### Copy a file

```aura
fn main() -> i64 {
    let argv = args()
    if argv.len < 3 {
        eprintln("usage: copy <src> <dst>")
        return 2
    }
    let data = match read_file(vec_get(argv, 1)) {
        Ok(d) => d,
        Err(e) => {
            eprintln("read: " + e)
            return 1
        }
    }
    if write_file(vec_get(argv, 2), data) { 0 } else {
        eprintln("write failed")
        1
    }
}
```

### Count stdin bytes

```aura
fn main() -> i64 {
    let input = read_stdin()
    println(str_from_int(input.len))
    0
}
```

### Run a helper and forward its exit code

```aura
fn main() -> i64 {
    let code = exec("git status")
    if code < 0 {
        eprintln("could not spawn")
        return 1
    }
    code                                  // propagate the child's code
}
```

`exec` splits on whitespace — no shell expansion, no quoting. For
shell features, `exec("cmd /c ...")` on Windows.

### Sum a vector

```aura
fn sum(v: vec<i64>) -> i64 {
    let mut total = 0
    let mut i = 0
    while i < v.len {
        total = total + vec_get(v, i)
        i = i + 1
    }
    total
}
```
