# Feature Specification: Semver Publishing Pipeline

**Feature Branch**: `048-semver-publishing-pipeline`
**Created**: 2026-07-03
**Status**: Approved
**Input**: Cargo.toml workspace version has drifted from the git tag (0.5.0 in Cargo.toml, v0.7.0 tagged). No crates.io publishing is configured. No automated version bump exists. `repository` field still points to old org URL. This spec closes all three gaps.

## Purpose

This spec defines the complete semver lifecycle for Traverse: how versions move from Cargo.toml through git tag to crates.io, enforced by CI so no release can ship without the workspace version, tag, and published crates being in sync.

Three problems to solve:

1. **Version drift**: `Cargo.toml` version must match the git tag on every release. Currently they differ.
2. **No crates.io publishing**: All six crates are unpublished. `publish` is not set, meaning `cargo publish` would attempt to publish without a gate.
3. **No automation**: Version bumps are manual and error-prone. A release script must handle bump → commit → tag → publish atomically.

## User Scenarios and Testing

### User Story 1 — Cargo.toml version always matches the git tag (Priority: P0)

As a downstream developer, I want `cargo add traverse-runtime@0.7.0` to work so that I can depend on Traverse without cloning the source.

**Acceptance Scenarios**:

1. **Given** a release is tagged `v0.7.0`, **When** `grep version Cargo.toml` runs, **Then** it prints `0.7.0`.
2. **Given** the workspace version is `0.7.0`, **When** `git tag --list | grep v0.7.0` runs, **Then** the tag exists.
3. **Given** CI runs on a tag push, **When** the tag does not match `Cargo.toml` version, **Then** the `version-guard` CI job fails with a clear error message.

### User Story 2 — All six crates published to crates.io on every release tag (Priority: P1)

As a downstream developer, I want all Traverse crates available on crates.io so I can use them without a git dependency.

**Acceptance Scenarios**:

1. **Given** a `v*` tag is pushed, **When** the `publish` CI job runs, **Then** all six crates are published in dependency order: `traverse-contracts` → `traverse-registry` → `traverse-runtime` → `traverse-mcp` → `traverse-cli` → `traverse-expedition-wasm`.
2. **Given** a crate is already published at that version (idempotent re-run), **When** `cargo publish` runs, **Then** the job skips cleanly rather than failing.
3. **Given** any crate fails to publish, **When** the job reports, **Then** it names the failing crate and exits non-zero — no partial publish silently succeeds.

### User Story 3 — Version bump is a single script, not manual edits (Priority: P1)

As a release engineer, I want `bash scripts/ci/bump_version.sh <new-version>` to update Cargo.toml, commit, tag, and push so that version bumps have no manual file-editing step.

**Acceptance Scenarios**:

1. **Given** current version `0.7.0`, **When** `bash scripts/ci/bump_version.sh 0.8.0` runs, **Then** `Cargo.toml` contains `version = "0.8.0"`, a commit `chore: bump version to v0.8.0` exists, and tag `v0.8.0` is created locally.
2. **Given** an invalid semver string like `foo`, **When** the script runs, **Then** it exits non-zero with a clear error before making any changes.
3. **Given** the working tree has uncommitted changes, **When** the script runs, **Then** it exits non-zero and makes no changes.

### User Story 4 — repository URL and crate metadata are correct (Priority: P0)

As a crates.io consumer, I want the `repository` and `homepage` fields in published crates to point to `traverse-framework/Traverse`.

**Acceptance Scenarios**:

1. **Given** `Cargo.toml` workspace package, **When** `grep repository Cargo.toml` runs, **Then** it prints `https://github.com/traverse-framework/Traverse`.
2. **Given** a published crate on crates.io, **When** the metadata is inspected, **Then** `repository` resolves to `https://github.com/traverse-framework/Traverse`.

## Functional Requirements

- **FR-001**: `[workspace.package]` in `Cargo.toml` MUST have `repository = "https://github.com/traverse-framework/Traverse"`.
- **FR-002**: All six crates MUST have `publish = true` (or omit the field, which defaults to true) and `repository.workspace = true`.
- **FR-003**: CI MUST include a `version-guard` job that runs on every `push` (branch and tag) and fails if `Cargo.toml` version does not match the tag when a `v*` tag is present.
- **FR-004**: CI MUST include a `publish` job triggered only on `push` of a `v*` tag, publishing crates in dependency order.
- **FR-005**: `scripts/ci/bump_version.sh <semver>` MUST validate input is a valid semver string before making any changes.
- **FR-006**: `bump_version.sh` MUST refuse to run on a dirty working tree.
- **FR-007**: `bump_version.sh` MUST update only `[workspace.package] version` in `Cargo.toml` — no other files.
- **FR-008**: After `bump_version.sh`, running `cargo build` MUST succeed without manual intervention.

## Non-Functional Requirements

- **NFR-001**: Publish order MUST respect crate dependency graph — no crate is published before its dependencies.
- **NFR-002**: The `publish` CI job MUST use `CARGO_REGISTRY_TOKEN` from GitHub Actions secrets — no token in code or scripts.
- **NFR-003**: Publish is idempotent — re-running on an already-published version is a no-op, not an error.

## Files Governed

- `Cargo.toml` (repository URL, version)
- `scripts/ci/bump_version.sh` (new)
- `.github/workflows/ci.yml` (version-guard job, publish job)
- `docs/release-process.md` (new — documents the full release sequence)
