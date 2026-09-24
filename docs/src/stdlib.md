# Standard Library

The prelude is a set of compiler-injected builtins — no `import` needed.
A user-defined function with the same name shadows the builtin.

## I/O

| Builtin | Effect |
|---------|--------|
| `print(s: str)` | write `s` to stdout |
| `println(s: str)` | write `s` + `\n` to stdout |
| `eprint(s: str)` | write `s` to stderr |
| `eprintln(s: str)` | write `s` + `\n` to stderr |
| `read_stdin() -> str` | read all of stdin |
| `exit(code: i64) -> !` | terminate the process |

## Files

| Builtin | Effect |
|---------|--------|
| `read_file(path: str) -> Result<str, str>` | whole file as `str`; `Err(msg)` on failure |
| `write_file(path: str, data: str) -> bool` | write `data`; `true` on success |

Paths are relative to the process working directory. Both engines
resolve them identically.

## Math

| Builtin | Effect |
|---------|--------|
| `sqrt(x: f64) -> f64` | square root |
| `f64_from_int(n) -> f64` | signed-integer → `f64` conversion |

## Vectors

`vec<T>` is a growable buffer, `{ptr, len, cap}` internally. The
builtins are generic — `T` is inferred from first use:

```aura
let v = vec_new()          // vec<?T>
vec_push(v, 10)            // v: vec<i64>
vec_set(v, 0, 42)
let x = vec_get(v, 0)      // bounds-checked; exits 101 on OOB
let last = vec_pop(v)      // removes & returns last element
v.len                      // usize — element count
v.cap                      // usize — capacity (grows by doubling)
v.ptr                      // *const u8 — raw data pointer
```

Aggregate elements work: `vec<str>` and `vec<MyStruct>` are supported.
`vec_get`/`vec_set`/`vec_pop` on an out-of-bounds index abort the
process with exit code `101` (same in the interpreter).

## Strings

`str` is a `{ptr, len}` byte slice — `.len` is the **byte** length:

```aura
let s = "ab" + "cd"        // heap-allocated concat
s.len                      // usize — byte length
s.ptr                      // *const u8 — data pointer
s == "abcd"                // content equality
```

| Builtin | Effect |
|---------|--------|
| `str_get(s: str, i) -> i64` | byte at index `i` (0–255); exits 101 on OOB |
| `str_slice(s: str, a, b) -> str` | bytes `a..b`; exits 101 on bad range |
| `str_from_int(n) -> str` | decimal rendering (`-42` → `"-42"`) |
| `str_from_bool(b: bool) -> str` | `"true"` / `"false"` |
| `str_from_byte(b) -> str` | one-byte string from the low 8 bits of `b` |
| `str_from_f64(x: f64) -> str` | fixed `%.6f` formatting (`3.5` → `"3.500000"`, `-0.0` → `"-0.000000"`, huge → `"±inf"`, NaN → `"nan"`) |

Ordering comparisons (`<`, `>`, `<=`, `>=`) on `str` are a type error.

## Process & environment

| Builtin | Effect |
|---------|--------|
| `args() -> vec<str>` | command-line arguments (argv[0] included) |
| `env(name: str) -> str` | environment variable, `""` when unset |
| `exec(cmd: str) -> i64` | spawn program + whitespace-split args; exit code, `-1` if spawn fails |

`exec` invokes the program directly (no shell): `exec("prog arg1 arg2")`.

## Semantics notes

- Runtime calls have fixed C signatures — no varargs anywhere.
- `exit` works in both engines: compiled code calls `ExitProcess`;
  the interpreter propagates `Escape::Exit` to the top level.
- The interpreter is the behavioral reference — differential tests
  assert compiled and interpreted output agree byte-for-byte on every
  `testsuite/` scenario.
