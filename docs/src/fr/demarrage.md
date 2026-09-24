# Démarrage — Aura en 15 minutes

Ce guide vous emmène de zéro à un programme Aura qui tourne.
English version: `docs/src/getting-started.md`.

## Installation

- **Assistant d'installation** : téléchargez `AuraSetup-*.exe` depuis la
  page GitHub Releases du projet et lancez-le (Suivant → Suivant →
  Terminer). Il ajoute `aura` à votre PATH — pas besoin d'admin.
- **Manuel** : dézippez `aura-*-windows-x86_64.zip`, ajoutez le dossier
  au PATH.

Vérifiez que ça marche :

```text
aura --version
```

Windows peut afficher un avertissement SmartScreen au premier lancement
(binaires alpha non signés) — « Informations complémentaires » →
« Exécuter quand même ».

## Votre premier programme

```text
aura new hello
cd hello
aura run
```

`aura new` a créé :

```text
hello/
  aura.toml        [package] name = "hello"
  src/main.aura    fn main() -> i64 { println("Hello, Aura!")  return 0 }
```

- `aura run` interprète `main` directement — itération rapide.
- `aura build` produit un vrai `build/hello.exe` natif.
- `aura check` vérifie les types sans exécuter.

## Le tour en 60 secondes

```aura
// fonctions
fn square(x: i64) -> i64 { x * x }

// erreurs — Result + `?`
fn might_fail(ok: bool) -> Result<i64, str> {
    if ok { Ok(42) } else { Err("nope") }
}

fn main() -> i64 {
    // variables — inférées, immuables par défaut
    let name = "world"
    let mut count = 0                      // `mut` rend assignable
    count = count + 1

    // types : i64 i32 u8 usize f64 bool str vec<T> Result<T,E>
    let pi: f64 = 3.14159
    let scores = vec_new()                 // vec<i64> — inféré…
    vec_push(scores, 7)                    // …depuis ce push

    // contrôle — `if` est une expression
    let label = if count > 0 { "oui" } else { "non" }
    while count < 10 { count = count + 1 }

    println("hello " + name)               // concaténation
    println(str_from_int(square(7)))       // int → str
    println(str_from_int(vec_get(scores, 0)))
    match might_fail(true) {
        Ok(v) => println(str_from_int(v)),
        Err(e) => eprintln(e),
    }
    return 0                                // code de sortie
}
```

Les instructions sont séparées par des retours à la ligne — pas de
points-virgules. La dernière expression d'une fonction est sa valeur de
retour (`return` marche aussi).

## Batteries incluses

| Besoin | Builtin |
|--------|---------|
| afficher | `print(s)`, `println(s)`, `eprintln(s)` |
| chaînes → nombres | `str_get(s, i)`, `str_slice(s, a, b)` |
| nombres → chaînes | `str_from_int(n)`, `str_from_f64(x)` (`%.6f`) |
| vecteurs | `vec_new()`, `vec_push`, `vec_get`, `vec_set`, `vec_pop` |
| fichiers | `read_file(path)` → `Result`, `write_file(path, s)` → `bool` |
| système | `args()`, `env(name)`, `exec(cmd)`, `read_stdin()` |
| maths | `sqrt(x)`, `f64_from_int(n)` |
| processus | `exit(code)` |

Référence complète : `aura doc stdlib`.

## Quand quelque chose ne va pas

Chaque erreur a un code — consultez-le :

```text
aura doc E2101     # explique « if utilisé comme valeur exige else »
aura doc errors    # l'index complet
```

## Pour aller plus loin

- `docs/language.md` — la référence complète du langage
- `docs/effective-aura.md` — idiomes et anti-patrons
- `bench/*.aura`, `testsuite/` — 50+ programmes d'exemple réels
