# Build a multi-file project

**Goal.** An app that calls functions from a `mathlib` dependency
package — the same layout as `testsuite/46-multifile`.

## Layout

```text
app/
  aura.toml          # declares the package + its deps
  src/main.aura
  mathlib/
    aura.toml        # dep package manifest
    src/lib.aura     # dep's public functions
```

## `app/aura.toml`

```toml
[package]
name = "app"

[dependencies]
mathlib = { path = "mathlib" }
```

## `app/mathlib/aura.toml`

```toml
[package]
name = "mathlib"
```

## `app/mathlib/src/lib.aura`

A dependency's `src/lib.aura` holds its public items — functions are
callable by dependents without `use`:

```aura
fn square(x: i64) -> i64 { x * x }

fn sumsq(v: vec<i64>) -> i64 {
    let mut t = 0
    let mut i = 0
    while i < v.len {
        t = t + square(vec_get(v, i))
        i = i + 1
    }
    t
}
```

## `app/src/main.aura`

```aura,ignore
fn main() -> i64 {
    let v = vec_new()
    vec_push(v, 3)
    vec_push(v, 4)
    println(str_from_int(sumsq(v)))   // 3*3 + 4*4 = 25
    0
}
```

Run from `app/`: `aura run` → prints `25`. The project loader reads
`aura.toml`, pulls `mathlib`, and links `lib.aura`'s items into scope.

## Going further

- Multiple deps: each gets `name = { path = "..." }` and its own
  `src/lib.aura`.
- Split the *main* package too — more `.aura` files under `src/`
  share the same namespace.
- `testsuite/47-libapp` shows structs + enums crossing packages;
  `48-bigger` composes two deps.
