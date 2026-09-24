# Construire un projet multi-fichiers

**Objectif.** Une app qui appelle des fonctions d'une dépendance
`mathlib` — le même layout que `testsuite/46-multifile`.

## Layout

```text
app/
  aura.toml          # déclare le package + ses deps
  src/main.aura
  mathlib/
    aura.toml        # manifeste du package dep
    src/lib.aura     # fonctions publiques de la dep
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

Le `src/lib.aura` d'une dépendance contient ses items publics — les
fonctions sont appelables par les dépendants sans `use` :

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

Depuis `app/` : `aura run` → affiche `25`. Le chargeur de projet lit
`aura.toml`, tire `mathlib`, et lie les items de `lib.aura` dans le scope.

## Pour aller plus loin

- Plusieurs deps : chacune a `name = { path = "..." }` et son propre
  `src/lib.aura`.
- Découper aussi le package *principal* — d'autres fichiers `.aura`
  sous `src/` partagent le même namespace.
- `testsuite/47-libapp` montre structs + enums traversant les packages ;
  `48-bigger` compose deux deps.
