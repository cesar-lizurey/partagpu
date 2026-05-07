🇫🇷 [Version française](RELEASING.md)

# Cutting a new release

Two independent artifacts, two separate tags.

## Tauri app (`.deb` + `.AppImage` + GitHub Release)

Workflow: [`.github/workflows/release.yml`](../.github/workflows/release.yml).
Trigger: tag `vX.Y.Z` (e.g. `v1.7.1`).

1. Bump the version in **the three places that must stay in sync**:
   - [`src-tauri/Cargo.toml`](../src-tauri/Cargo.toml) — `[package]` section → `version`
   - [`src-tauri/tauri.conf.json`](../src-tauri/tauri.conf.json) — `version` key
   - [`package.json`](../package.json) — `version` key
2. Commit the bump: `git commit -am "Bump version to 1.7.1"`
3. Tag and push:
   ```bash
   git tag v1.7.1
   git push origin main v1.7.1
   ```
4. CI runs `cargo fmt --check`, `cargo clippy -- -D warnings`, then `cargo test --all-targets --locked`. Any clippy warning or test failure aborts the release. On green it builds the `.deb` + `.AppImage` and creates the GitHub Release.
5. Verify the release on the [releases page](https://github.com/cesar-lizurey/partagpu/releases).

To avoid publishing the release immediately, mark the tag as a prerelease by editing the release after creation (the workflow defaults to `prerelease: false`).

## Python package (`pip install partagpu`)

Workflow: [`.github/workflows/pypi.yml`](../.github/workflows/pypi.yml).
Trigger: tag `python-vX.Y.Z` (e.g. `python-v1.4.1`). Independent of the Tauri version — the Python package follows its own cadence.

1. Bump in [`python/pyproject.toml`](../python/pyproject.toml) → `version` key in the `[project]` section.
2. Commit: `git commit -am "Python: bump to 1.4.1"`
3. Tag and push:
   ```bash
   git tag python-v1.4.1
   git push origin main python-v1.4.1
   ```
4. CI builds and publishes to PyPI via *trusted publishing* (no API token needed; the `pypi` environment must be declared on [PyPI → Manage account → Publishing](https://pypi.org/manage/account/publishing/)).

## Before pushing a tag

- `npx tsc --noEmit` must pass.
- `cargo fmt --all --check` then `cargo clippy --all-targets --all-features --locked -- -D warnings` (from `src-tauri/`) must pass. CI promotes every clippy warning to an error — what passes locally passes in CI.
- `cargo test` (from `src-tauri/`) must pass — including the integration suite (`cargo test --test peer_api_e2e`). The release workflow already runs this gate, so a failed local run means the CI will reject the tag.
- `npx tauri build --bundles deb` once locally to confirm the bundle assembles cleanly before consuming a CI run.
