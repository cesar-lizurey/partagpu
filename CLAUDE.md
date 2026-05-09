# PartaGPU — consignes pour Claude

## Versionning

Toutes les versions sont **synchronisées** sur le numéro de l'app. Quand on bumpe l'app, on bumpe les quatre fichiers ensemble :

- `package.json`
- `src-tauri/tauri.conf.json`
- `src-tauri/Cargo.toml` (crate `partagpu`)
- `src-tauri/helper/Cargo.toml` (crate `partagpu-helper`)

Le helper a beau ne pas changer à chaque release, on le bumpe quand même pour qu'un seul numéro identifie un état du repo. `cargo check` après les edits pour rafraîchir `Cargo.lock` (sinon le commit ne touche pas le lock alors qu'il devrait).

Le **package Python** dans `python/pyproject.toml` a sa propre série (`1.6.x`, taggée `python-vX.Y.Z`) parce qu'il publie sur PyPI indépendamment de l'app desktop.

## Commits

- Pas de mention `Co-Authored-By: Claude` dans les commits (préférence globale, voir `~/.claude/CLAUDE.md`).
- Le seul auteur visible est `cesar-lizurey`.

## Pré-push : reproduire la CI localement

```bash
cargo clippy -p partagpu --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt --all --check --manifest-path src-tauri/Cargo.toml
npm run typecheck
```

## Tags GitHub Actions

- `vX.Y.Z` → `release.yml` build le `.deb`
- `python-vX.Y.Z` → `pypi.yml` publie le package Python sur PyPI

Le push d'un commit sur `main` seul ne déclenche **rien** : il faut le tag pour qu'un workflow démarre.
