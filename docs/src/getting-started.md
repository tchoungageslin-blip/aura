# Getting Started — Aura in 15 minutes

This guide takes you from zero to a running Aura program. French
version: `docs/src/fr/demarrage.md`.

## Install

- **Setup wizard**: download `AuraSetup-*.exe` from the project's GitHub
  Releases page and run it (Next → Next → Finish). It adds `aura` to
  your PATH — no admin needed.
- **Manual**: unzip `aura-*-windows-x86_64.zip`, add the folder to PATH.

Check it works:

```text
aura --version
```

Windows may show a SmartScreen prompt on first run (unsigned alpha
binaries) — "More info" → "Run anyway".

## Your first program

```text
aura new hello
cd hello
aura run
```

`aura new` created:

```text
hello/
  aura.toml        [package] name = "hello"
  src/main.aura    fn main() -> i64 { println("Hello, Aura!")  return 0 }
```

- `aura run` interprets `main` directly — fast to iterate.
- `aura build` produces a real native `build/hello.exe`.
- `aura check` type-checks without running.

## The 60-second tour

```aura
// functions
fn square(x: i64) -> i64 { x * x }

// errors — Result + `?`
fn might_fail(ok: bool) -> Result<i64, str> {
    if ok { Ok(42) } else { Err("nope") }
}

fn main() -> i64 {
    // variables — inferred, immutable by default
    let name = "world"
    let mut count = 0                      // `mut` makes it assignable
    count = count + 1

    // types: i64 i32 u8 usize f64 bool str vec<T> Result<T,E>
    let pi: f64 = 3.14159
    let scores = vec_new()                 // vec<i64> — inferred…
    vec_push(scores, 7)                    // …from this push

    // control flow — `if` is an expression
    let label = if count > 0 { "yes" } else { "no" }
    while count < 10 { count = count + 1 }

    println("hello " + name)               // string concat
    println(str_from_int(square(7)))       // int → str
    println(str_from_int(vec_get(scores, 0)))
    match might_fail(true) {
        Ok(v) => println(str_from_int(v)),
        Err(e) => eprintln(e),
    }
    return 0                                // exit code
}
```

Statements are separated by newlines — no semicolons. The last
expression of a function is its return value (`return` also works).

## Batteries included

| Need | Builtin |
|------|---------|
| print | `print(s)`, `println(s)`, `eprintln(s)` |
| strings → numbers | `str_get(s, i)`, `str_slice(s, a, b)` |
| numbers → strings | `str_from_int(n)`, `str_from_f64(x)` (`%.6f`) |
| vectors | `vec_new()`, `vec_push`, `vec_get`, `vec_set`, `vec_pop` |
| files | `read_file(path)`, `write_file(path, s)` → `Result` |
| system | `args()`, `env(name)`, `exec(cmd)`, `read_stdin()` |
| math | `sqrt(x)`, `f64_from_int(n)` |
| process | `exit(code)` |

Full reference: `aura doc stdlib`.

## When something goes wrong

Every error has a code — look it up:

```text
aura doc E2101     # explains "if used as a value requires else"
aura doc errors    # the full index
```

## Next steps

- `docs/language.md` — the complete language reference
- `docs/effective-aura.md` — idioms and anti-patterns
- `bench/*.aura`, `testsuite/` — 50+ real example programs
