# Construire un KV store

**Objectif.** Une mini base clé-valeur en mémoire pilotée par des
commandes stdin :

```text
SET name aura      -> ok
GET name           -> aura
GET missing        -> (none)
COUNT              -> 1
```

Le motif — scan d'octets, tokenisation, boucle de commandes — est le
cœur de la plupart des vrais outils Aura. (La version complète avec
`DEL` est dans
[exemples/43-kv](https://tchoungageslin-blip.github.io/aura/examples.html).)

## Helpers — mêmes idiomes de scan que l'outil CLI

```aura
fn line_at(s: str, i: usize) -> usize {
    // index of the next '\n' at-or-after i (s.len if none)
    let mut j = i
    while j < s.len && str_get(s, j) != 10 {
        j = j + 1
    }
    j
}

fn eq_at(s: str, i: usize, w: str) -> bool {
    // does s contain w starting at i?
    i + w.len <= s.len && str_slice(s, i, i + w.len) == w
}
```

## Stockage — deux vecs parallèles

Pas besoin de hash map à cette échelle : `keys[i]` va avec `vals[i]`,
et « absent » = la sentinelle `keys.len` :

```aura
fn find_key(keys: vec<str>, k: str) -> usize {
    let mut i = 0
    while i < keys.len {
        if vec_get(keys, i) == k { return i }
        i = i + 1
    }
    keys.len   // sentinel: not found
}
```

## Boucle de commandes

`SET ` / `GET ` sont matchés comme préfixes — l'espace finale du motif
signifie que la clé commence juste après. Le programme complet :

```aura
fn line_at(s: str, i: usize) -> usize {
    let mut j = i
    while j < s.len && str_get(s, j) != 10 {
        j = j + 1
    }
    j
}

fn eq_at(s: str, i: usize, w: str) -> bool {
    i + w.len <= s.len && str_slice(s, i, i + w.len) == w
}

fn find_key(keys: vec<str>, k: str) -> usize {
    let mut i = 0
    while i < keys.len {
        if vec_get(keys, i) == k { return i }
        i = i + 1
    }
    keys.len
}

fn main() -> i64 {
    let inp = read_stdin()
    let keys = vec_new()
    let vals = vec_new()
    let mut i = 0
    while i < inp.len {
        let e = line_at(inp, i)
        if eq_at(inp, i, "SET ") {
            // "SET k v" — k ends at the next space, v runs to eol
            let mut sp = i + 4
            while sp < e && str_get(inp, sp) != 32 { sp = sp + 1 }
            let key = str_slice(inp, i + 4, sp)
            let val = str_slice(inp, sp + 1, e)
            let idx = find_key(keys, key)
            if idx == keys.len {
                vec_push(keys, key)
                vec_push(vals, val)
            } else {
                vec_set(vals, idx, val)
            }
            println("ok")
        } else if eq_at(inp, i, "GET ") {
            let key = str_slice(inp, i + 4, e)
            let idx = find_key(keys, key)
            if idx == keys.len {
                println("(none)")
            } else {
                println(vec_get(vals, idx))
            }
        } else if eq_at(inp, i, "COUNT") {
            println(str_from_int(keys.len))
        }
        i = e + 1
    }
    0
}
```

Notes :

- `vec_set(vals, idx, v)` modifie en place — pas besoin de `let mut`
  sur le vec puisqu'on mute via le builtin, pas via le binding.
- `find_key` retourne `keys.len` quand absent — un idiome sans Option
  utilisé partout dans la testsuite.
- `vec<str>` est inféré depuis `vec_push(keys, key)` — `vec_new()`
  seul ne peut pas choisir `T` (`E2111`).

## Pour aller plus loin

- `DEL k` — marquer un slot vide avec `vec_set(keys, idx, "")`.
- Persistance : sur `SAVE`, joindre les paires et
  `write_file("kv.db", out)`.
- Le scénario complet ajoute `DEL` et `COUNT` : `testsuite/43-kv`.
