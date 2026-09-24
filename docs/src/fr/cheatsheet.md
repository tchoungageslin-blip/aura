# Aide-mémoire

Tout le langage sur une page. Chaque bloc ci-dessous est un programme
complet et compilable. [English version](../cheatsheet.md).

## Squelette

```aura
fn main() -> i64 {
    println("hello")
    0                     // expression finale = valeur de retour
}
```

## Variables & types

```aura
fn main() -> i64 {
    let x = 41            // i64, immuable, inféré
    let mut y: i32 = 1    // mutable, annoté
    y = y + 1
    let f: f64 = 1.5
    let b = true          // bool
    let s = "text"        // str
    let v: vec<i64> = vec_new()   // vec<T> exige un T concret ici
    x                     // retourne 41
}
```

Types : `i64` `i32` `i16` `i8` `u64` `u32` `u16` `u8` `f64` `f32`
`bool` `str` `vec<T>` `Result<T,E>` `()` et `struct`/`enum`
utilisateur. Pas d'élargissement implicite : `x as i32` convertit.

## Contrôle

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
    loop { break }        // boucle inconditionnelle
    classify(i)
}
```

`if` est une expression : `let m = if a > b { a } else { b }`.

## Fonctions

```aura
fn add(a: i64, b: i64) -> i64 { a + b }   // expr finale, pas de return

fn add_one(x: i64) -> i64 { x + 1 }

fn main() -> i64 {
    add_one(add(20, 1))                   // les appels se composent
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

Les littéraux de struct exigent des parenthèses en tête d'expression.

## Enums & match

```aura
enum Shape { Circle(f64), Rect(f64, f64), Empty }

fn tag(s: Shape) -> i64 {
    match s {
        Circle(r) => 1,
        Rect(w, h) => 2,
        _ => 0,           // match doit être exhaustif
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

`expr?` déballe `Ok` ou retourne le `Err` immédiatement — uniquement
dans une fonction qui retourne `Result`.

## Strings

```aura
fn main() -> i64 {
    let s = "ab" + "cd"                   // concaténation
    println(str_from_int(s.len))          // 4 (octets)
    println(str_from_int(str_get(s, 0)))  // 97 = 'a'
    println(str_slice(s, 1, 3))           // "bc"
    println(str_from_int(42))             // "42"
    0
}
```

`==` compare par contenu. Les comparaisons (`<`) sur `str` sont
rejetées — et `println` n'accepte que `str`, donc `str_from_int(n)`
pour afficher un nombre.

## Vecteurs

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

## E/S, fichiers, args, env

```aura
fn main() -> i64 {
    let a = args()                        // vec<str>, argv[0] inclus
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

Liste complète : [Bibliothèque standard](../stdlib.md).

## Projets & FFI

`use` + dépendances `aura.toml` pour les programmes multi-fichiers —
voir [La toolchain](../toolchain.md). `extern "C"` déclare des
fonctions natives (types scalaires uniquement) :

```aura,ignore
extern "C" { fn GetTickCount64() -> u64 }
```

## Pièges

- Aucune conversion implicite — `str_from_int(n)` avant de concaténer,
  et `println` n'accepte que `str`.
- `let` est immuable ; `let mut` pour réassigner.
- Les instructions finissent au saut de ligne (ASI) ; une expression
  finale sans `;` est la valeur de retour.
- `match` doit être exhaustif — `_` pour le cas par défaut.
- En compilé, un dépassement de borne ou arithmétique termine avec le
  code 101 ; `aura interp` produit une erreur propre.
