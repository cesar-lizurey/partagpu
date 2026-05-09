🇬🇧 [English version](RELEASING.en.md)

# Publier une nouvelle version

Le projet produit deux artefacts indépendants, qui sont publiés via deux *tags* distincts.

## Application Tauri (`.deb`, `.AppImage` et *GitHub Release*)

Le *workflow* concerné est [`.github/workflows/release.yml`](../.github/workflows/release.yml).
Il se déclenche sur un *tag* de la forme `vX.Y.Z` (par exemple `v1.6.1`).

1. Mettre à jour la version dans les **trois fichiers qui doivent rester synchronisés** :
   - [`src-tauri/Cargo.toml`](../src-tauri/Cargo.toml) — section `[package]` → clé `version` ;
   - [`src-tauri/tauri.conf.json`](../src-tauri/tauri.conf.json) — clé `version` ;
   - [`package.json`](../package.json) — clé `version`.
2. Créer un commit pour cette montée de version : `git commit -am "Bump version to 1.7.1"`.
3. Créer le *tag* et le pousser :
   ```bash
   git tag v1.7.1
   git push origin main v1.7.1
   ```
4. La CI exécute d'abord `cargo fmt --check`, puis `cargo clippy -- -D warnings`, puis `cargo test --all-targets --locked`. Le moindre avertissement de Clippy ou échec de test interrompt la publication. Sinon, la CI compile le `.deb` et l'`.AppImage`, puis crée la *GitHub Release*.
5. Vérifier la publication sur la [page des releases](https://github.com/cesar-lizurey/partagpu/releases).

Pour ne pas publier tout de suite, marquer la publication comme préliminaire (*prerelease*) en éditant la *release* après sa création (par défaut, la CI la marque comme finale, c'est-à-dire `prerelease: false`).

## Paquet Python (`pip install partagpu`)

Le *workflow* concerné est [`.github/workflows/pypi.yml`](../.github/workflows/pypi.yml).
Il se déclenche sur un *tag* de la forme `python-vX.Y.Z` (par exemple `python-v1.4.1`). Ce *tag* est indépendant de la version de l'application Tauri — le paquet Python évolue à son propre rythme.

1. Mettre à jour la version dans [`python/pyproject.toml`](../python/pyproject.toml), clé `version` de la section `[project]`.
2. Créer le commit : `git commit -am "Python : bump to 1.4.1"`.
3. Créer le *tag* et le pousser :
   ```bash
   git tag python-v1.4.1
   git push origin main python-v1.4.1
   ```
4. La CI compile et publie sur PyPI via la *publication de confiance* (*trusted publishing*) — il n'y a pas de jeton d'API à configurer. L'environnement `pypi` doit en revanche être déclaré dans [PyPI → Manage account → Publishing](https://pypi.org/manage/account/publishing/).

## Avant de pousser un *tag*

- `npx tsc --noEmit` doit passer sans erreur.
- `cargo fmt --all --check`, puis `cargo clippy --all-targets --all-features --locked -- -D warnings` (depuis `src-tauri/`), doivent passer. La CI traite chaque avertissement de Clippy comme une erreur : ce qui passe en local passera en CI.
- `cargo test` (depuis `src-tauri/`) doit passer, y compris la suite d'intégration (`cargo test --test peer_api_e2e`). Le *workflow* de publication applique déjà cette vérification : un échec en local signifie que la CI rejettera le *tag*.
- Lancer `npx tauri build --bundles deb` au moins une fois en local pour vérifier que le paquet s'assemble correctement, avant de consommer une exécution de CI.
