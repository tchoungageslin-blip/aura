# Build a CLI tool

**Goal.** `grep-lite WORD` — read stdin, print only the lines that
contain `WORD`, exit `0` on success and `2` on bad usage.

```sh
Get-Content app.log | aura run grep-lite.aura ERROR
```

## Step 1 — shape of the program

`args()` returns a `vec<str>` — index 0 is the program name. We need
`argv[1]`:

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

`eprintln` writes to stderr; `return 2` is the conventional
"misuse" exit code (grep does the same).

## Step 2 — split stdin into lines

Bytes are addressed with `str_get(s, i)` (returns the byte value),
slices with `str_slice(s, lo, hi)`. Newline is byte `10`:

```aura
fn line_end(s: str, i: usize) -> usize {
    let mut j = i
    while j < s.len && str_get(s, j) != 10 {
        j = j + 1
    }
    j
}
```

## Step 3 — substring test

Sliding window over the line — same idiom as `line_end`:

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

## Step 4 — assemble

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

Run it: `Get-Content file | aura run grep-lite.aura needle` — or
`aura interp grep-lite.aura` for the instant-feedback version while
iterating.

## Going further

- Count matches and print `"matches: " + str_from_int(n)` at the end.
- Exit `1` when nothing matched, like real grep.
- Case-insensitive match: compare `str_get` byte-by-byte with a
  fold-ASCII-uppercase helper.
