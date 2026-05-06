# Publier une nouvelle version

Deux artefacts indépendants, deux tags séparés.

## Application Tauri (`.deb` + `.AppImage` + GitHub Release)

Workflow : [`.github/workflows/release.yml`](../.github/workflows/release.yml).
Déclencheur : tag `vX.Y.Z` (ex. `v1.6.1`).

1. Mettre à jour la version au **trois endroits qui doivent rester synchronisés** :
   - [`src-tauri/Cargo.toml`](../src-tauri/Cargo.toml) — section `[package]` → `version`
   - [`src-tauri/tauri.conf.json`](../src-tauri/tauri.conf.json) — clé `version`
   - [`package.json`](../package.json) — clé `version`
2. Commit la bump : `git commit -am "Bump version to 1.6.1"`
3. Tagger et pousser :
   ```bash
   git tag v1.6.1
   git push origin main v1.6.1
   ```
4. La CI build le `.deb` + `.AppImage` puis crée la GitHub Release.
5. Vérifier la release sur la [page des releases](https://github.com/cesar-lizurey/partagpu/releases).

Pour ne pas créer la release publique tout de suite, marquer le tag en
prerelease en éditant la release après création (la CI la met en
`prerelease: false` par défaut).

## Package Python (`pip install partagpu`)

Workflow : [`.github/workflows/pypi.yml`](../.github/workflows/pypi.yml).
Déclencheur : tag `python-vX.Y.Z` (ex. `python-v1.4.1`). Indépendant de
la version Tauri — le paquet Python évolue à son propre rythme.

1. Bump dans [`python/pyproject.toml`](../python/pyproject.toml) → clé
   `version` de la section `[project]`.
2. Commit : `git commit -am "Python : bump to 1.4.1"`
3. Tagger et pousser :
   ```bash
   git tag python-v1.4.1
   git push origin main python-v1.4.1
   ```
4. La CI build et publie sur PyPI via *trusted publishing* (pas de token
   API à configurer ; l'environnement `pypi` doit être déclaré sur
   [PyPI → Manage account → Publishing](https://pypi.org/manage/account/publishing/)).

## Avant de pousser un tag

- `npx tsc --noEmit` doit passer sans erreur.
- `cargo test --manifest-path src-tauri/Cargo.toml` doit passer (notamment
  les tests crypto).
- `npx tauri build --bundles deb` au moins une fois en local pour
  vérifier que le bundle s'assemble bien avant de consommer un run de CI.
