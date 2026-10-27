# Feature Specification: Registry Sync CLI

**Feature Branch**: `055-registry-sync`
**Created**: 2026-07-06
**Status**: Approved — v1.1.0 model-rights inspection amend (#1727, registry spec 026) approved 2026-10-27
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
- **FR-014** *(v1.1.0, #1727)*: Sync MUST keep, unchanged in the synced state, the registry spec 026 (`026-model-rights-compliance`) fields that the index carries: each record's `ai` declaration (including every `ai.models` rights record), `status`, `model_usage`, and `revocation`. Index rows that predate spec 026 MUST still sync.
- **FR-015** *(v1.1.0, #1727)*: `traverse-cli registry inspect <capability-id> --workspace <id> [--namespace] [--version] [--json]` MUST show, for every matching synced version (newest first), the lifecycle status (`active|deprecated|revoked`, taken from the index `status` or else the legacy `deprecated` flag), the revocation record when one exists, and for each object-shaped `ai.models` entry: `spdx_expression`, `attribution_required`, `commercial_use`, `redistribution`, `derivatives`, the derived `usage_class`, the pinned LICENSE and NOTICE files with their sha256, `derivation`, `data_obligations`, and `verification.status`. A field the publisher did not declare MUST print as `not stated`; it MUST NOT print as `none` or as a permission value. When no synced record matches, the command MUST fail with `registry_capability_not_found`.
- **FR-016** *(v1.1.0, #1727)*: A legacy `string[]` `ai.models` entry MUST be shown as `not recorded (legacy shape)` and MUST NOT cause an error.
- **FR-017** *(v1.1.0, #1727)*: `traverse-cli registry search` MUST accept `--status <active|deprecated|revoked>` and `--model-commercial-use <allowed|forbidden|conditional|unknown>`. The second keeps capabilities with at least one object-shaped model declaring that value; legacy and undeclared models never match. These combine with the spec 025 `--commercial-use`/`--redistribution` capability filters, which keep their meaning. Search JSON records MUST include `status`. Inspect and search output is declared metadata only: no value is turned into a permission decision here, which stays with runtime policy.
- **FR-018** *(v1.1.0, #1727)*: Inspect and search output MUST be deterministic: the same synced state gives byte-identical output.

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
6. **Given** a synced index with a revoked version carrying a revocation record, **When** `registry inspect` runs, **Then** that version shows `status: revoked` and its revocation record, and every model rights field is shown.
7. **Given** a model reference without `data_obligations`, **When** it is inspected, **Then** `data_obligations` shows `not stated`.
8. **Given** synced versions with `allowed` and `conditional` model commercial-use rights, **When** `registry search --model-commercial-use conditional` runs, **Then** only the `conditional` version is returned.

## Out of Scope

- Hosted registry APIs.
- Team-shared private registry sources.
- Eager artifact mirroring during sync.
- Publication of new capabilities; that is governed by `056-capability-publish`.
- Runtime enforcement of revoked versions or `conditional` rights, and contract-signature verification of the rights fields (tracked in #1728 and #1729).
