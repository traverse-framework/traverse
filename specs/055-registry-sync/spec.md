# Feature Specification: Registry Sync CLI

**Feature Branch**: `055-registry-sync`
**Created**: 2026-07-06
**Status**: Approved
**Input**: GitHub issue #542, registry decision log entries 6-7, registry specs `001-registry-foundation` and `003-index-release-pipeline`, and Traverse spec `054-public-scope-registry-ref`.

## Purpose

Traverse MUST provide `traverse-cli registry sync` so a workspace can consume the public capability registry without any live network dependency at runtime execution. The command fetches the latest published registry index artifact, validates it, and writes durable local public-tier state.

## Requirements

- **FR-001**: `traverse-cli registry sync` MUST fetch the latest `index.json` asset from the latest `index-v*` GitHub Release in `traverse-framework/registry`.
- **FR-002**: The command MAY use the unauthenticated GitHub Releases API for the default public registry source.
- **FR-003**: The command MUST validate the fetched index schema before writing local state.
- **FR-004**: The command MUST validate that every indexed capability has non-empty `namespace`, `id`, `version`, `digest`, `artifact_url`, and `deprecated` fields.
- **FR-005**: The command MUST write synced records into durable workspace state as the public registry tier.
- **FR-006**: Runtime and app-registration resolution MUST read synced public records from local durable state and MUST NOT live-fetch from GitHub or any hosted registry service.
- **FR-007**: Sync MUST be atomic: a failed fetch, malformed index, invalid record, or failed write MUST NOT corrupt the last valid synced state.
- **FR-008**: Sync MUST preserve enough metadata for diagnosis and traceability: source repo, source release tag, index version, source commit when present, synced-at timestamp, record count, and validation status.
- **FR-009**: If no synced state exists and public-tier resolution is requested, resolution MUST fail with a stable actionable error indicating that `traverse-cli registry sync` is required.
- **FR-010**: If synced state is stale, resolution MAY continue against the last valid synced state, but CLI output MUST expose the staleness metadata when available.
- **FR-011**: Sync MUST fetch the index only. It MUST NOT download all WASM artifacts eagerly.
- **FR-012**: Artifact download for `registry_ref` dependencies happens at app-registration time under `054-public-scope-registry-ref`.
- **FR-013**: The command MUST support a JSON output mode suitable for CI and downstream automation.

## Command Shape

```bash
traverse-cli registry sync --workspace local-default --json
```

Successful JSON output MUST include at least:

```json
{
  "status": "synced",
  "source": "traverse-framework/registry",
  "release_tag": "index-v42",
  "index_version": 42,
  "record_count": 12,
  "workspace": "local-default"
}
```

## Acceptance Scenarios

1. **Given** a published registry release with valid `index.json`, **When** `traverse-cli registry sync --json` runs, **Then** durable public-tier state is written and the JSON output reports the source release and record count.
2. **Given** the previous sync wrote valid local state, **When** a later sync fetches malformed JSON, **Then** the command fails and the previous state remains available.
3. **Given** sync has completed, **When** runtime resolution of a public capability runs, **Then** no live network call is made.
4. **Given** sync has never completed, **When** public-tier resolution is requested, **Then** the error code and message tell the operator to run `traverse-cli registry sync`.
5. **Given** an app manifest uses `registry_ref`, **When** registration starts after sync, **Then** it resolves against the local public tier produced by sync.

## Out of Scope

- Hosted registry APIs.
- Team-shared private registry sources.
- Eager artifact mirroring during sync.
- Publication of new capabilities; that is governed by `056-capability-publish`.
