# Feature Specification: Capability Publish CLI

**Feature Branch**: `056-capability-publish`
**Created**: 2026-07-06
**Status**: Approved
**Input**: GitHub issue #543, registry decision log entry 9, registry specs `001-registry-foundation` and `002-capability-validation`, and Traverse specs `051-registry-extraction` and `054-public-scope-registry-ref`.

## Purpose

Traverse MUST provide `traverse-cli capability publish` as a governed PR automation command for submitting capability publication candidates to `traverse-framework/registry`. The command automates local validation, digest collection, branch preparation, and PR creation. It MUST NOT bypass deterministic registry CI or explicit human review.

## Requirements

- **FR-001**: `traverse-cli capability publish` MUST validate the candidate capability contract locally before creating any registry PR.
- **FR-002**: The command MUST refuse content sourced from a bundle or manifest marked `scope: private`.
- **FR-003**: The command MUST reject contracts with invalid schema, invalid semver, missing owner metadata, missing artifact metadata, or a missing digest before any PR is opened.
- **FR-004**: The command MUST compute or verify the artifact digest before preparing the publication record.
- **FR-005**: The command MUST map the candidate into the registry repo path `capabilities/<namespace>/<id>/<version>/contract.json`.
- **FR-006**: The command MUST refuse to overwrite an existing published path in a local registry checkout before opening a PR.
- **FR-007**: The command MUST create a dedicated branch in `traverse-framework/registry` for the publication candidate.
- **FR-008**: The command MUST open a pull request against `traverse-framework/registry` using `git` and `gh`, with a body that includes the relevant governing specs and validation evidence.
- **FR-009**: The command MUST preserve manual approval: no successful local command result may publish a capability without the registry PR being reviewed, approved, merged, and indexed by registry CI.
- **FR-010**: If PR creation fails after local branch or file creation, the command MUST report the partial state and next cleanup or retry command. It MUST NOT silently delete user work.
- **FR-011**: The command MUST support a dry-run mode that performs validation and reports the planned registry path, branch name, and PR title without writing to the registry repo.
- **FR-012**: The command MUST support JSON output suitable for automation.
- **FR-013**: The command MUST be idempotent enough for retry: re-running after a network or PR creation failure MUST detect existing prepared state and either reuse it or report the conflict explicitly.

## Command Shape

```bash
traverse-cli capability publish \
  --contract contracts/examples/traverse-starter/capabilities/process/contract.json \
  --artifact artifacts/process-agent.wasm \
  --registry-repo ../registry \
  --json
```

Successful JSON output after PR creation MUST include at least:

```json
{
  "status": "pr_opened",
  "registry_repo": "traverse-framework/registry",
  "branch": "publish/traverse-starter.process-1.0.0",
  "registry_path": "capabilities/traverse-starter/traverse-starter.process/1.0.0/contract.json",
  "pull_request_url": "https://github.com/traverse-framework/registry/pull/123"
}
```

## Acceptance Scenarios

1. **Given** a valid public capability contract and artifact, **When** `traverse-cli capability publish --json` runs, **Then** it opens a registry PR at the correct path with validation evidence in the body.
2. **Given** the source bundle is `scope: private`, **When** publish runs, **Then** it fails before branch or PR creation with an actionable private-scope refusal.
3. **Given** local validation fails, **When** publish runs, **Then** no registry branch or PR is created.
4. **Given** the target registry path already exists, **When** publish runs, **Then** it fails with an immutable-version conflict before modifying the registry checkout.
5. **Given** PR creation fails after a branch is prepared, **When** the command exits, **Then** JSON output reports the branch and files that remain for retry or cleanup.

## Out of Scope

- Auto-merging registry PRs.
- Replacing registry CI validation.
- Third-party publisher onboarding and namespace claiming.
- Hosted publish APIs.
