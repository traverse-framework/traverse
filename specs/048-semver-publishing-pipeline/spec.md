# Feature Specification: Semver Publishing Pipeline

**Feature Branch**: `048-semver-publishing-pipeline`
**Created**: 2026-07-03
**Amended**: 2026-09-09
**Status**: Approved
**Version**: 1.2.0
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

### User Story 5 — Web embedder publication uses npm Trusted Publishing (Priority: P0)

As a web consumer, I want `traverse-embedder-web` releases published from a
reviewed `web-v<version>` tag using GitHub OIDC, so that publication is
reproducible and never depends on a long-lived npm token.

**Acceptance Scenarios**:

1. **Given** `web-v0.9.0` is pushed and the package version is `0.9.0`, **When**
   the web publish workflow runs, **Then** it publishes exactly `0.9.0` with a
   provenance attestation.
2. **Given** a `web-v*` tag whose version differs from `package.json`, **When**
   the workflow runs, **Then** its version guard fails before any publication.
3. **Given** the same web tag is rerun after publication, **When** npm reports
   that version already exists, **Then** the workflow succeeds without replacing
   the published artifact.

## Functional Requirements

- **FR-001**: `[workspace.package]` in `Cargo.toml` MUST have `repository = "https://github.com/traverse-framework/Traverse"`.
- **FR-002**: All six crates MUST have `publish = true` (or omit the field, which defaults to true) and `repository.workspace = true`.
- **FR-003**: CI MUST include a `version-guard` job that runs on every `push` (branch and tag) and fails if `Cargo.toml` version does not match the tag when a `v*` tag is present.
- **FR-004**: CI MUST include a `publish` job triggered only on `push` of a `v*` tag, publishing crates in dependency order.
- **FR-005**: `scripts/ci/bump_version.sh <semver>` MUST validate input is a valid semver string before making any changes.
- **FR-006**: `bump_version.sh` MUST refuse to run on a dirty working tree.
- **FR-007**: `bump_version.sh` MUST restrict its edits to `Cargo.toml` and `Cargo.lock` — no other files. In `Cargo.toml` it updates `[workspace.package] version` and the matching `[workspace.dependencies]` path-crate `version =` pins; in `Cargo.lock` it updates the `version = "…"` line of each workspace path crate only (`[[package]]` entries named `traverse-*` that carry no `source` line). If any other file shows as changed, the script MUST abort without committing.
- **FR-008**: After `bump_version.sh`, running `cargo build` MUST succeed without manual intervention, and `cargo metadata --locked` (equivalently `cargo build --locked`) MUST succeed with no further change to `Cargo.lock`.
- **FR-009**: `bump_version.sh` MUST stage `Cargo.toml` and `Cargo.lock` together in the single `chore: bump version to v<version>` commit, so the tree is tag-ready in one step with no follow-up lockfile-sync commit.
- **FR-010**: CI MUST fail a `Cargo.toml`/`Cargo.lock` version drift before the `publish` job runs. The `version-guard` job MUST run `cargo metadata --locked` on every `push`, `pull_request`, and `v*` tag, and `publish` MUST depend on `version-guard`.
- **FR-011**: A dedicated workflow MUST trigger only for `web-v*` tags and publish `packages/web/TraverseEmbedder` through npm Trusted Publishing with `id-token: write` and `contents: read`; it MUST NOT reference `NPM_TOKEN` or `NODE_AUTH_TOKEN`.
- **FR-012**: The web workflow MUST use a committed npm lockfile, run `npm ci`, validate that `web-v<version>` exactly matches `package.json`, build, test, and publish with `npm publish --provenance --access public`.
- **FR-013**: Web publication reruns MUST be idempotent: an already-published exact version is a successful no-op. The release runbook MUST document the OIDC model, `web-v<version>` tag creation, and post-publish verification.

## Non-Functional Requirements

- **NFR-001**: Publish order MUST respect crate dependency graph — no crate is published before its dependencies.
- **NFR-002**: The `publish` CI job MUST use `CARGO_REGISTRY_TOKEN` from GitHub Actions secrets — no token in code or scripts.
- **NFR-003**: Publish is idempotent — re-running on an already-published version is a no-op, not an error.

## Files Governed

- `Cargo.toml` (repository URL, version)
- `Cargo.lock` (workspace path-crate `version` entries only, kept in step with
  `Cargo.toml` by `bump_version.sh`; not registered as a spec-alignment
  `governs` prefix, since routine dependency updates touch this file
  independently of the release pipeline)
- `scripts/ci/bump_version.sh` (new)
- `.github/workflows/ci.yml` (version-guard job, publish job)
- `.github/workflows/web-embedder-publish.yml` (web tag publication)
- `packages/web/TraverseEmbedder/package.json` and `package-lock.json` (web release identity and locked dependencies)
- `packages/web/TraverseEmbedder/.npmrc` (web tag prefix)
- `docs/release-process.md` and `docs/web-embedder-npm-publish-runbook.md` (release documentation)

## Amendment History

### 1.1.0 — 2026-09-05

**Owner**: Traverse maintainers (issue #1236). **Rationale**: the v0.10.0 release
shipped `Cargo.toml` at `0.10.0` while `Cargo.lock` still pinned the workspace
crates at `0.9.1`, breaking every `--locked` build on `main` and forcing a
follow-up sync PR (#1233). Branch/PR CI never ran `--locked`, so the drift was
invisible until tag time.

**Changes**: FR-007 widened from "only `Cargo.toml`" to "`Cargo.toml` and
`Cargo.lock` only", with `Cargo.lock` edits scoped to workspace path-crate
version lines. FR-008 now also requires `cargo metadata --locked` to succeed
with no lockfile change. New FR-009 (single tag-ready commit contains both
files) and FR-010 (`version-guard` runs `cargo metadata --locked` on every
push, PR, and tag; `publish` depends on it). `Cargo.lock` documented under
Files Governed but deliberately left out of the `governs` prefix list. No
change to the publish flow, crate list, or version-bump interface.

### 1.2.0 — 2026-09-09

**Owner**: Traverse maintainers (issue #1316). **Rationale**: the public web
embedder needs a governed release path that does not rely on an expiring
personal npm credential.

**Changes**: adds npm Trusted Publishing as a second, independent publication
target. `web-v<version>` tags bind an exact package version, a committed lockfile
makes installation reproducible, and provenance is attached by npm. This is
additive: the crate publishing interface and `v<version>` tags are unchanged.
