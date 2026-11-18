# Feature Specification: Private Registry Source Override

**Feature Branch**: `523-private-registry-source-override`
**Created**: 2026-07-28
**Status**: Approved (2026-11-17)
**Version**: 1.0.0
**Approval**: Owner-approved 2026-11-17 (#1901). Implemented in #843.
**Input**: GitHub issue #842, `traverse-framework/registry` `docs/decision-log.md` (private-registry brainstorm, same session), spec `055-registry-sync`, spec `056-capability-publish`.

## Purpose

`055-registry-sync` and `056-capability-publish` hardcode `traverse-framework/registry` as the only registry source `traverse-cli` can sync from or publish to (`055`'s own "Out of Scope" section explicitly excludes "Team-shared private registry sources"). The repo owner asked for something structurally similar to npm's private-registry story: a team should be able to point `traverse-cli` at their own registry source -- a private (or otherwise non-default) GitHub repo that adopts the same `capabilities/<namespace>/<id>/<version>/contract.json` layout and index-build CI as `traverse-framework/registry` -- using the same commands and mechanism, not a separate system.

This spec covers making the source repo configurable and reachable when private (token auth). It does **not** cover a host-agnostic registry protocol independent of GitHub -- that broader version was considered and explicitly deferred in favor of this smaller, immediately achievable one (see the registry repo's decision log for the tradeoff discussion).

## Requirements

### Functional Requirements

- **FR-001**: `traverse-cli registry sync` MUST accept an optional `--source-repo <owner/repo>` flag. When omitted, behavior is unchanged: it defaults to `traverse-framework/registry`.
- **FR-002**: `traverse-cli capability publish` MUST accept a way to override the PR target repo (today hardcoded via `DEFAULT_REGISTRY_REPO`, independently of `registry sync`'s source) -- this is a second, distinct hardcoded constant on a distinct code path and must not be assumed fixed by FR-001 alone.
- **FR-003**: The registry-index fetch path (`CurlGitHubRegistryIndexFetcher`) MUST support an optional bearer token, sourced from an environment variable or CLI flag (implementer's choice, documented either way), sent as `Authorization: Bearer <token>` -- required to reach a private GitHub repo's Releases API, which 404s/rate-limits without auth.
- **FR-004**: When no override and no token are supplied, behavior MUST be byte-for-byte identical to today's hardcoded, unauthenticated public-registry path -- this is an additive capability, not a behavior change to the default.
- **FR-005**: `--json` output for `registry sync` MUST continue to report the actual source repo used (already true today via the `source` field), so a synced state's provenance is inspectable regardless of which source it came from.
- **FR-006**: Help text (`help_registry_sync`, `help_registry`, `help_capability_publish`) MUST document the new flag(s).

### Key Entities

- **Registry source**: an `owner/repo` GitHub identifier that structurally implements the same layout as `traverse-framework/registry` (capability contract tree + index-release CI). Not a new entity type in code -- today's `CurlGitHubRegistryIndexFetcher.source_repo` field already models this; it just needs to be reachable via a config path other than a hardcoded constant.

## Acceptance Scenarios

1. **Given** no `--source-repo` flag, **When** `registry sync --workspace w --json` runs, **Then** it fetches from `traverse-framework/registry`, identical to today.
2. **Given** `--source-repo acme-corp/internal-registry`, **When** `registry sync --workspace w --json` runs against a public repo with that layout, **Then** it fetches from that repo instead, and the JSON output's `source` field reflects it.
3. **Given** `--source-repo acme-corp/internal-registry` where that repo is private, and no token supplied, **When** sync runs, **Then** it fails with a clear, actionable error (not a silent empty result) -- exact error message left to the implementer, but it must distinguish "repo not found/no access" from other fetch failures.
4. **Given** the same private repo and a valid token supplied, **When** sync runs, **Then** it succeeds identically to the public case.
5. **Given** `capability publish` with a registry-repo override, **When** it opens its PR, **Then** the PR targets the overridden repo, not the hardcoded default.

## Out of Scope

- A host-agnostic registry protocol independent of GitHub Releases (npm-style pluggable backends: self-hosted static file servers, Artifactory, etc.) -- deferred; see `traverse-framework/registry`'s decision log for why the smaller GitHub-repo-based version was chosen first.
- Any hosted, multi-tenant private-registry service.
- Per-namespace multi-source routing (syncing from several sources at once and merging) -- one source per sync invocation, same as today.

## Note on approval

This spec was drafted by an agent (Claude Code) working from `traverse-framework/registry` issue #842 and a same-session brainstorm with the repo owner about the private-registry *scope* decision (GitHub-repo-based, not a new protocol). Per this repo's spec-then-implementation precedent (and `registry`'s own no-self-approval-of-specs rule, which this agent defaults to here absent confirmation this repo's rule differs), this spec stays **Draft** and unregistered in `specs/governance/approved-specs.json` pending the repo owner's explicit sign-off -- implementation proceeds under it in the interim, consistent with how `registry`'s own spec `007-workflow-registry-traversal` was implemented while still Draft.

**Approved 2026-11-17**: the repo owner signed off on this spec (#1901), and it is now registered in `specs/governance/approved-specs.json`.
