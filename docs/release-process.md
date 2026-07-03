# Release Process

Governed by `spec 048-semver-publishing-pipeline`.

## Prerequisites

- `CARGO_REGISTRY_TOKEN` set as a GitHub Actions secret (one-time setup by repo owner)
- Working tree is clean (`git status` shows nothing)
- All PRs for the release are merged to `main`
- `cargo test` passes on `main`

## Steps

### 1. Bump the version

```bash
bash scripts/ci/bump_version.sh <new-version>
```

Example: `bash scripts/ci/bump_version.sh 1.0.0`

This updates `[workspace.package] version` in `Cargo.toml`, commits
`chore: bump version to v<new-version>`, and creates a local tag `v<new-version>`.

The script refuses to run on a dirty working tree or with an invalid semver string.

### 2. Push the commit and tag

```bash
git push origin main
git push origin v<new-version>
```

Pushing the tag triggers the `publish` CI job automatically.

### 3. CI publishes to crates.io

The `publish` job in `.github/workflows/publish.yml` fires on every `v*` tag push.
It runs `scripts/ci/publish_crates.sh` which publishes all six crates in dependency order:

```
traverse-contracts → traverse-registry → traverse-runtime
  → traverse-mcp → traverse-cli → traverse-expedition-wasm
```

Publishing is idempotent — re-running on an already-published version is a no-op.

### 4. Verify

- Check [crates.io/crates/traverse-runtime](https://crates.io/crates/traverse-runtime) for the new version
- Confirm the GitHub release was created (manual step — create via GitHub UI from the pushed tag)
- Run `bash scripts/ci/v1_gate_check.sh` to verify all v1 gate conditions (for v1.0.0 only)

## Version guard

The `version-guard` CI job runs on every push. On tag pushes it verifies the tag version matches
`Cargo.toml`. A tag that does not match fails CI before publishing starts.

## Rollback

crates.io does not support yanking from CI. If a bad version is published:

1. `cargo yank --version <version> -p <crate>` for each affected crate
2. Fix the issue on main
3. Bump to a new patch version and re-release

## What NOT to do

- Do not edit `Cargo.toml` version manually — use `bump_version.sh`
- Do not push a `v*` tag without a matching `Cargo.toml` version — `version-guard` will block it
- Do not set `CARGO_REGISTRY_TOKEN` in any file — only in GitHub Actions secrets
