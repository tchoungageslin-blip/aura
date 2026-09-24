# Build a KV store

**Goal.** A tiny in-memory key-value store driven by stdin commands:

```text
SET name aura      -> ok
GET name           -> aura
GET missing        -> (none)
COUNT              -> 1
```

The pattern — byte scanning, tokenizing, a command loop — is the core
of most real Aura tools. (The full version with `DEL` lives in
[examples/43-kv](https://tchoungageslin-blip.github.io/aura/examples.html).)

## Helpers — same scanning idioms as the CLI tool

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

## Storage — two parallel vecs

No hash map needed at this scale: `keys[i]` pairs with `vals[i]`,
and "not found" is the `keys.len` sentinel:

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

## Command loop

`SET ` / `GET ` are matched as prefixes — the trailing space in the
pattern means the key starts right after it. The whole program:

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

Notes:

- `vec_set(vals, idx, v)` updates in place — `let mut` is *not*
  needed on the vec since we mutate through the builtin, not the
  binding.
- `find_key` returns `keys.len` when absent — a cheap Option-free
  idiom used all over the testsuite.
- `vec<str>` is inferred from `vec_push(keys, key)` — `vec_new()`
  alone can't pick a `T` (`E2111`).

## Going further

- `DEL k` — mark a slot empty with `vec_set(keys, idx, "")`.
- Persist: on `SAVE`, join pairs and `write_file("kv.db", out)`.
- The full scenario adds `DEL` and `COUNT`: `testsuite/43-kv`.
