# Tutoriels

Constructions guidées — chaque page mène un petit programme réel du
besoin au code fonctionnel. Chaque bloc `aura` est compilé par la CI :
ce que tu lis est ce qui tourne.

| Tutoriel | Tu construis | Tu apprends |
|----------|--------------|-------------|
| [Outil CLI](cli-tool.md) | `grep-lite` — filtre stdin avec args | `args`, `read_stdin`, slices, codes de sortie |
| [Traitement de fichiers](file-processing.md) | compteur de lignes qui écrit un rapport | `read_file`, `write_file`, `match` sur `Result` |
| [KV store](kv-store.md) | serveur `SET/GET` sur stdin | tokenisation, `vec`s parallèles, boucle de commandes |
| [Projet multi-fichiers](project.md) | app + dépendance `mathlib` | `aura.toml`, `src/lib.aura`, packages |

Versions anglaises : [tutorials](../).

Plus de programmes complets : la [galerie d'exemples](https://tchoungageslin-blip.github.io/aura/examples.html)
liste les 50 scénarios de validation avec leur source.
