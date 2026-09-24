# Build a file processor

**Goal.** Read `input.txt`, count its lines, and write
`report.txt` containing `lines: N`. A first look at `Result`
and the file builtins.

## Signatures

```text
read_file(path: str)              -> Result<str, str>
write_file(path: str, data: str)  -> bool
```

`read_file` returns `Err` when the file is missing/unreadable —
`match` handles both branches. `write_file` is a plain `bool`:
`true` on success.

## Count lines

```aura
fn count_lines(text: str) -> i64 {
    let mut lines = 0
    let mut i = 0
    while i < text.len {
        if str_get(text, i) == 10 {   // '\n'
            lines = lines + 1
        }
        i = i + 1
    }
    lines
}
```

## Read → transform → write

```aura
fn count_lines(text: str) -> i64 {
    let mut lines = 0
    let mut i = 0
    while i < text.len {
        if str_get(text, i) == 10 {   // '\n'
            lines = lines + 1
        }
        i = i + 1
    }
    lines
}

fn main() -> i64 {
    match read_file("input.txt") {
        Ok(text) => {
            let report = "lines: " + str_from_int(count_lines(text))
            if !write_file("report.txt", report) {
                eprintln("cannot write report.txt")
                return 1
            }
            println("wrote report.txt")
            0
        }
        Err(e) => {
            eprintln("cannot read input.txt: " + e)
            1
        }
    }
}
```

Notes:

- `str + str` concatenates, but numbers are **not** auto-converted —
  `str_from_int` is required (`E2100` otherwise).
- `!` negates the `bool` from `write_file`; there is no `Err` to match.
- Both `match` arms return `i64` — the `match` *is* the function value.

## Going further

- Words too: count runs of non-space bytes.
- Append the byte count: `"bytes: " + str_from_int(text.len)`.
- Take the paths from `args()` instead of hardcoding them.
