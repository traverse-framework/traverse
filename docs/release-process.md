# Traverse Release Process

Governed by spec `048-semver-publishing-pipeline`.

Use the release helper from a clean `main` checkout after CI is green:

1. Run `bash scripts/ci/bump_version.sh <version>`.
2. Run `git push origin main`.
3. Run `git push origin v<version>`.
4. Confirm the CI publish job starts automatically from the `v<version>` tag push.
5. Verify crates.io lists all six Traverse crates at the new version:
   `traverse-contracts`, `traverse-registry`, `traverse-runtime`,
   `traverse-mcp`, `traverse-cli-rs`, and `traverse-expedition-wasm`.

The version argument is `MAJOR.MINOR.PATCH` without a leading `v`. The helper
refuses invalid versions, dirty working trees, and pre-existing local release
tags. It updates only `[workspace.package] version` in `Cargo.toml`, creates the
commit `chore: bump version to v<version>`, and creates the local tag
`v<version>`. It does not push commits or tags.

On a tag push, the `version-guard` CI job compares the tag without the leading
`v` to the workspace version in `Cargo.toml`. Branch and pull-request runs pass
without a release tag, while mismatched release tags fail before publishing.

The tag-only `publish` CI job runs after `version-guard`, repository checks, and
coverage pass. It runs `bash scripts/ci/publish_crates.sh` with
`CARGO_REGISTRY_TOKEN` from GitHub Actions secrets. The script dry-runs each
crate immediately before publishing it, publishes crates in dependency order,
and treats an already-uploaded crate version as success so reruns are
idempotent.
