# Feature Specification: Public Scope and Registry References

**Feature Branch**: `054-public-scope-registry-ref`
**Created**: 2026-07-06
**Status**: Approved
**Input**: GitHub issue #548, registry decision log entries 20-21, and registry companion spec `006-public-scope-and-identity`.

## Purpose

Traverse MUST reconcile local bundle registry scope with the public capability registry before `registry sync` and `capability publish` are implemented. Public capability records come only from `traverse-framework/registry`; local application bundles remain a private overlay unless a capability is explicitly published through the governed registry PR path.

This spec governs the Traverse-side runtime, registry, CLI, and application manifest behavior. The registry repo owns publisher identity fields such as `namespace` and `owner`; Traverse owns local resolution tiers, app registration, and execution behavior.

## Requirements

- **FR-001**: `RegistryScope::Public` and `RegistryScope::Private` MUST remain resolution tiers, separate from registry publisher identity fields such as `namespace` and `owner`.
- **FR-002**: The public tier of durable local registry state MUST be populated only by `traverse-cli registry sync` from the `traverse-framework/registry` published index.
- **FR-003**: Local app or bundle registration MUST reject `scope: public` once registry sync exists, with an actionable error directing users to `traverse-cli capability publish` for public publication or `scope: private` for local testing.
- **FR-004**: `scope: private` MUST continue to populate the private overlay and MUST continue to win under prefer-private lookup semantics.
- **FR-005**: A private overlay record MAY shadow a synced public identity, but registration MUST emit a warning that the local private record overrides the public record for prefer-private lookup.
- **FR-006**: `traverse-cli capability publish` MUST refuse content sourced from private bundles. Private scope acts as a publish-refusal latch.
- **FR-007**: Component manifests MUST support exactly one capability source per component: either local bundled artifacts via `contract_path` or a public registry dependency via `registry_ref`.
- **FR-008**: `registry_ref` MUST contain non-empty `namespace`, `id`, and `version_range` fields.
- **FR-009**: `contract_path` with `wasm_binary_path` and `wasm_digest` MUST keep its existing meaning: the component bundles its own capability and app registration writes it into the private overlay.
- **FR-010**: `registry_ref` MUST resolve only against the sync-populated public tier. It MUST NOT fall back to live network calls, local public registrations, or private overlay records.
- **FR-011**: App registration MUST fetch artifacts for `registry_ref` dependencies at registration time, verify the published digest, and store them in a local content-addressed cache before execution.
- **FR-012**: Runtime execution MUST never fetch registry indexes or artifacts from the network.
- **FR-013**: Registry reference failures MUST be deterministic and actionable, including at least: registry never synced, no matching version, dependency yanked, artifact download failed, and digest mismatch.
- **FR-014**: The design MUST NOT prevent future multi-source sync for team-private registries, but this spec only approves the single public source `traverse-framework/registry`.

## Manifest Shape

Local bundled component:

```json
{
  "capability_id": "traverse-starter.process",
  "contract_path": "contracts/process/contract.json",
  "wasm_binary_path": "artifacts/process-agent.wasm",
  "wasm_digest": "fnv1a64:..."
}
```

Public registry component:

```json
{
  "registry_ref": {
    "namespace": "traverse-starter",
    "id": "traverse-starter.process",
    "version_range": "^1.0.0"
  }
}
```

## Acceptance Scenarios

1. **Given** a synced public index containing `traverse-starter.process`, **When** an app manifest uses a matching `registry_ref`, **Then** app registration resolves the public record, fetches and verifies the artifact, and stores it locally before execution.
2. **Given** no registry sync has run, **When** an app manifest uses `registry_ref`, **Then** app registration fails with a stable error indicating that `traverse-cli registry sync` is required.
3. **Given** a local app bundle declares `scope: public`, **When** app registration runs after registry sync support exists, **Then** registration rejects it with guidance to publish through the registry PR flow.
4. **Given** a private local record with the same identity as a synced public record, **When** prefer-private lookup runs, **Then** the private record wins and the registration path records a shadow warning.
5. **Given** execution starts for a registered app using `registry_ref`, **When** the runtime invokes the capability, **Then** it reads only local durable state and cached artifacts.

## Out of Scope

- Team-shared private registries and multi-source sync routing.
- Hosted registry APIs.
- Third-party namespace claiming and publisher onboarding.
- Implementing `traverse-cli registry sync` or `traverse-cli capability publish`; those are governed by follow-up specs.
