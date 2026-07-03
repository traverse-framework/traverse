# Feature Specification: Registry Extraction

**Feature Branch**: `051-registry-extraction`
**Created**: 2026-07-03
**Status**: Approved
**Input**: Brainstorm session establishing `traverse-framework/registry` as a dedicated repo for the public capability registry, extracted from this repo's `crates/traverse-registry`. Full narrative record: `traverse-framework/registry`'s `docs/decision-log.md`. Companion foundation spec: `traverse-framework/registry`'s `specs/001-registry-foundation/spec.md`.

## Purpose

This spec is the decision record required by this repo's constitution ("no unreviewed architectural change without a decision record when the change affects... registry behavior") for moving `crates/traverse-registry` — and the specs that govern it — out of this repo into `traverse-framework/registry`.

This spec does not redesign registry behavior. It documents what moves, what stays, what becomes an external dependency, and how governance is adopted going forward. Implementation (the actual crate/spec move) is tracked separately (see `## Follow-up Work`) and is blocked on this spec being approved.

## What Moves

- `crates/traverse-registry/` (the entire crate) moves to `traverse-framework/registry`.
- The specs that currently govern it move with it, adopted as that repo's own governing specs: `005-capability-registry`, `007-workflow-registry-traversal`, `011-event-registry`, `034-programmatic-registration`, `035-multi-agent-isolation`, `036-event-subscription-replay`, `037-semver-range-resolution`, `039-connector-plugin-architecture`, `041-workflow-composition-api`, `043-module-dependency-management`.
- Once the move lands, this repo's `specs/governance/approved-specs.json` entries for those spec ids are updated: their `governs` paths pointing at `crates/traverse-registry/` are removed (the path no longer exists in this repo), since those specs' substance is now governed by their re-adopted counterparts in `traverse-framework/registry`.

## What Stays

- `crates/traverse-contracts/` stays in this repo. It is the schema every consumer — this repo's runtime/CLI/MCP, and `traverse-framework/registry` — depends on symmetrically. Moving it would make this repo external to its own capability schema, which is backwards given the runtime is what actually executes against that schema.
- `crates/traverse-runtime/`, `crates/traverse-cli/`, `crates/traverse-mcp/` stay. `traverse-cli capability publish` (the PR-automation command described in `traverse-framework/registry`'s spec 001, User Story 1) is implemented here, not in the registry repo, since the CLI is this repo's control surface.

## Dependency Change

Once the extraction lands, `Cargo.toml` depends on `traverse-registry` as an external crate (crates.io or git dependency, consistent with this repo's existing spec `048-semver-publishing-pipeline`, which already treats `traverse-registry` as an independently-versioned, separately-published crate in the workspace's publish order: `traverse-contracts` → `traverse-registry` → `traverse-runtime` → `traverse-mcp` → `traverse-cli` → `traverse-expedition-wasm`). This spec does not change that publish order — it changes where `traverse-registry`'s source of truth lives.

## Governance Adoption

This repo adopts shared governance from `traverse-framework/.github` at version 1.0.0. Once this spec is approved:

- `CLAUDE.md`, `CONTRIBUTING.md`, `docs/quality-standards.md`, `docs/antipatterns.md`, `docs/compatibility-policy.md`, `docs/exception-process.md` are replaced with thin pointers to `traverse-framework/.github` rather than kept as independently-maintained local copies.
- `scripts/ci/spec_alignment_check.sh` continues to be vendored locally (CI needs it in-repo to run), but is treated as a pinned copy of the canonical version in `traverse-framework/.github`, not an independently-evolved fork.

## Follow-up Work (tracked separately, blocked on this spec's approval)

- Extract `crates/traverse-registry` source and its governing specs into `traverse-framework/registry` (tracked as a `needs-spec` ticket in that repo — see its ticket "Extract `traverse-registry` crate + content from `Traverse` into this repo").
- Update this repo's `Cargo.toml` to depend on the externally-published crate.
- Update `specs/governance/approved-specs.json` to remove the now-inapplicable `crates/traverse-registry/` `governs` entries from specs 005/007/011/034-037/039/041/043.
- Implement `traverse-cli capability publish` (PR-automation for publishing to the registry repo).

## Compatibility Impact

None to public runtime behavior — this is a source-location and governance change, not a behavioral one. `traverse-registry`'s public API surface is unaffected; only its repository of record changes. Downstream consumers depending on `traverse-registry` via crates.io are unaffected, since the crate continues to be published under the same name and semver line per spec `048-semver-publishing-pipeline`.
