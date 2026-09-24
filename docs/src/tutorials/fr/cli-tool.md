# Construire un outil CLI

**Objectif.** `grep-lite MOT` — lire stdin, n'afficher que les lignes
contenant `MOT`, sortir `0` en succès et `2` en mauvais usage.

```sh
Get-Content app.log | aura run grep-lite.aura ERROR
```

## Étape 1 — la forme du programme

`args()` retourne un `vec<str>` — l'index 0 est le nom du programme.
Il nous faut `argv[1]` :

```aura
fn main() -> i64 {
    let a = args()
    if a.len < 2 {
        eprintln("usage: grep-lite WORD")
        return 2
    }
    println("searching for: " + vec_get(a, 1))
    0
}
```

`eprintln` écrit sur stderr ; `return 2` est le code « mauvais usage »
conventionnel (comme grep).

## Étape 2 — découper stdin en lignes

Les octets s'adressent avec `str_get(s, i)` (valeur de l'octet), les
tranches avec `str_slice(s, lo, hi)`. Le saut de ligne est l'octet `10` :

```aura
fn line_end(s: str, i: usize) -> usize {
    let mut j = i
    while j < s.len && str_get(s, j) != 10 {
        j = j + 1
    }
    j
}
```

## Étape 3 — test de sous-chaîne

Fenêtre glissante sur la ligne — même idiome que `line_end` :

```aura
fn contains(line: str, w: str) -> bool {
    let mut k = 0
    while k + w.len <= line.len {
        if str_slice(line, k, k + w.len) == w {
            return true
        }
        k = k + 1
    }
    false
}
```

## Étape 4 — assembler

```aura
fn line_end(s: str, i: usize) -> usize {
    let mut j = i
    while j < s.len && str_get(s, j) != 10 {
        j = j + 1
    }
    j
}

fn contains(line: str, w: str) -> bool {
    let mut k = 0
    while k + w.len <= line.len {
        if str_slice(line, k, k + w.len) == w {
            return true
        }
        k = k + 1
    }
    false
}

fn main() -> i64 {
    let a = args()
    if a.len < 2 {
        eprintln("usage: grep-lite WORD")
        return 2
    }
    let word = vec_get(a, 1)
    let inp = read_stdin()
    let mut i = 0
    while i < inp.len {
        let j = line_end(inp, i)
        let line = str_slice(inp, i, j)
        if contains(line, word) {
            println(line)
        }
        i = j + 1
    }
    0
}
```

Lancer : `Get-Content fichier | aura run grep-lite.aura needle` — ou
`aura interp grep-lite.aura` pour l'itération instantanée.

## Pour aller plus loin

- Compter les matches et afficher `"matches: " + str_from_int(n)`.
- Sortir `1` quand rien ne matche, comme le vrai grep.
- Match insensible à la casse : comparer `str_get` octet par octet
  avec un helper de conversion ASCII majuscule.
