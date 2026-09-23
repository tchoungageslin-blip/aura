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
| `exit(code: i64) -> !` | terminate the process |

## Math

| Builtin | Effect |
|---------|--------|
| `sqrt(x: f64) -> f64` | square root (hardware/libm) |

## Vectors

`vec<T>` is a growable buffer, `{ptr, len, cap}` internally. The
builtins are generic — `T` is inferred from first use:

```aura
let v = vec_new()          // vec<?T>
vec_push(v, 10)            // v: vec<i64>
vec_set(v, 0, 42)
let x = vec_get(v, 0)      // bounds-checked; exits 101 on OOB
v.len                      // usize — element count
v.cap                      // usize — capacity (grows by doubling)
```

Aggregate elements work: `vec<str>` and `vec<MyStruct>` are supported.
`vec_get` on an out-of-bounds index aborts the process with exit
code `101` (compiled) or a `vec_oob` error (interpreter).

## Process

| Builtin | Effect |
|---------|--------|
| `args() -> vec<str>` | command-line arguments (argv[0] included) |
| `env(name: str) -> str` | environment variable, `""` when unset |

## Strings

`str` is `{ptr, len}`:

```aura
let s = "ab" + "cd"   // heap-allocated concat
s.len                 // usize — byte length
s.ptr                 // *const u8 — data pointer
s == "abcd"           // content equality
```

Ordering comparisons (`<`, `>`, `<=`, `>=`) on `str` are a type error.

## Semantics notes

- Runtime calls have fixed C signatures — no varargs anywhere.
- `exit` works in both engines: compiled code calls `ExitProcess`;
  the interpreter propagates `Escape::Exit` to the top level.
- The interpreter is the behavioral reference — differential e2e tests
  assert compiled and interpreted exit codes agree on every fixture.
