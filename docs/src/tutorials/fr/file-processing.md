# Construire un traitement de fichiers

**Objectif.** Lire `input.txt`, compter ses lignes, et écrire
`report.txt` contenant `lines: N`. Premier contact avec `Result`
et les builtins fichiers.

## Signatures

```text
read_file(path: str)              -> Result<str, str>
write_file(path: str, data: str)  -> bool
```

`read_file` retourne `Err` si le fichier est absent/illisible —
`match` gère les deux branches. `write_file` est un simple `bool` :
`true` en succès.

## Compter les lignes

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

## Lire → transformer → écrire

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

Notes :

- `str + str` concatène, mais les nombres ne sont **pas** convertis
  automatiquement — `str_from_int` est requis (`E2100` sinon).
- `!` inverse le `bool` de `write_file` ; il n'y a pas de `Err` à
  matcher.
- Les deux bras du `match` retournent `i64` — le `match` *est* la
  valeur de la fonction.

## Pour aller plus loin

- Les mots aussi : compter les suites d'octets non-espace.
- Ajouter le nombre d'octets : `"bytes: " + str_from_int(text.len)`.
- Prendre les chemins depuis `args()` au lieu de les coder en dur.
