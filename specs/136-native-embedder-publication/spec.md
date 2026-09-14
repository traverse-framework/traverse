# Feature Specification: Native Embedder Publication

**Feature Branch**: `claude/issue-1366-native-embedder-publication`
**Created**: 2026-09-13
**Status**: Approved (2026-09-13)
**Canonical governing ID**: `136-native-embedder-publication`
**Version**: 1.0.0
**Input**: Traverse #1366; Decision 83; Approved Specs `068-public-platform-embedder-packages`,
`048-semver-publishing-pipeline`, `074-swift-native-resource-control-certification`;
ADR-0070.

## Purpose

Define how the public Swift, Kotlin/Android, and .NET/WinUI embedder packages
are published so a downstream app can depend on them without a Traverse
checkout, extra GitHub Packages config, or a sidecar. Spec 068 already requires
a public versioned package (FR-001, FR-008). This successor names the
registries, ties package versions to the `v*` / crates line, requires
tag-triggered publish of all three native artifacts together, and requires
App-Refs to consume those published coordinates.

This spec does not implement host bridges, change `embedder-api/1.0.0`, or
classify Certified vs Preview (Spec 529).

## Relationship to existing specifications

| Specification | Relationship |
| --- | --- |
| 068-public-platform-embedder-packages | This spec makes FR-001 / FR-008 executable for Swift, Kotlin, and .NET public registries. |
| 048-semver-publishing-pipeline | Native packages follow the crates `v*` line, not npm's `web-v*` exception. |
| 073 / 074 / 076 | Engine, resource-control, and C-ABI rules are unchanged. XCFramework bytes must match the approved Apple profile (ADR-0070). |
| 529-production-platform-certification | Publication is Shipped. 529 classifies Certified vs Preview and must not be used as a Shipped gate. |
| 075-native-runtime-distribution-contract | Still governs `runtime.wasm` distribution, not host-package registries. |

## User scenarios and testing

### User Story 1 — Depend from language-default registries (Priority: P1)

An Android, Windows, or Apple app developer adds Traverse with only
`mavenCentral()`, nuget.org, or the GitHub-release XCFramework URL.

**Independent Test**: A clean clone of a consumer project with no sibling
Traverse repo resolves the package from that registry.

**Acceptance Scenarios**:

1. **Given** a `vX.Y.Z` tag that claims the native line shipped, **When** Maven
   Central is queried for `dev.traverse:traverse-embedder:X.Y.Z`, **Then** the
   artifact exists and an Android project with only `mavenCentral()` resolves it.
2. **Given** the same tag, **When** nuget.org is queried for `TraverseEmbedder`
   version `X.Y.Z`, **Then** the package exists and a WinUI project can
   `PackageReference` it without a vendor path.
3. **Given** the same tag, **When** `Package.swift` is read, **Then** its
   `TraverseSwiftHost` binary target URL is that tag's
   `TraverseSwiftHost.xcframework.zip` and the checksum matches the asset.

### User Story 2 — One number with the runtime tag (Priority: P1)

A reviewer compares crates.io, Maven, NuGet, and the XCFramework zip and sees
the same semver as the git tag.

**Independent Test**: For tag `vX.Y.Z`, all four native public artifacts that
this spec governs report `X.Y.Z`, or the tag's release notes do not claim the
native line shipped.

**Acceptance Scenarios**:

1. **Given** workspace version `X.Y.Z` and tag `vX.Y.Z`, **When** native
   publication runs, **Then** Maven, NuGet, and the XCFramework zip are all
   `X.Y.Z`.
2. **Given** a `v*` tag whose Maven, NuGet, or XCFramework artifact is missing
   or at another version, **When** a release is described as native-Shipped,
   **Then** that claim is invalid under this spec.
3. **Given** `v0.10.1` already shipped without Maven/NuGet, **When** this spec
   is applied, **Then** those packages are not published at `0.10.1`; they wait
   for a later `v*` that publishes all three.

### User Story 3 — App-Refs consume the published coordinates (Priority: P2)

Reference apps build native shells without `TRAVERSE_REPO` or
`vendor/traverse-embedder-dotnet`.

**Independent Test**: After the first native-Shipped tag, App-Refs Android,
WinUI, and Swift shells resolve packages from public registries.

## Functional requirements

- **FR-001**: The Shipped Kotlin artifact MUST be `dev.traverse:traverse-embedder`
  on Maven Central. GitHub Packages is not sufficient.
- **FR-002**: The Shipped .NET artifact MUST be `TraverseEmbedder` on nuget.org.
  GitHub Packages is not sufficient.
- **FR-003**: The Shipped Swift host binary MUST be `TraverseSwiftHost.xcframework.zip`
  attached to the same GitHub `v*` release that publishes Maven and NuGet.
  `Package.swift` MUST pin that URL and checksum.
- **FR-004**: Maven, NuGet, and XCFramework versions MUST equal the workspace /
  crates semver for that `v*` tag.
- **FR-005**: A `v*` tag MUST NOT claim the native embedder line shipped unless
  all three FR-001–FR-003 artifacts exist at that version. Kotlin/.NET
  implementation may merge earlier; the publish job waits so the three appear
  together.
- **FR-006**: Each published package MUST carry Spec 068 release evidence:
  package version, runtime-WASM digest, embedder-API / conformance version, and
  supported host versions.
- **FR-007**: Publication MUST be tag-triggered CI, idempotent if the version
  already exists, and must fail closed if the version does not match the tag.
- **FR-008**: After the first native-Shipped tag, `traverse-framework/reference-apps`
  MUST depend on those public coordinates for production Android, WinUI, and
  Swift shells. `TRAVERSE_REPO` path includes and `vendor/traverse-embedder-dotnet`
  MUST NOT remain the production resolve path.
- **FR-009**: The XCFramework attached for a tag whose Apple profile is wasmi
  2.0.0 MUST NOT be published until ADR-0070's physical iOS and macOS Spec 074
  fixtures for that engine version are recorded.

## Non-functional requirements

- **NFR-001 Traceability**: A downstream binary MUST be connectable to package
  version, runtime digest, and conformance result without private CI logs.
- **NFR-002 Compatibility**: Publication MUST NOT change `embedder-api/1.0.0`
  semantics.
- **NFR-003 Security**: Publish credentials MUST NOT be long-lived tokens in
  the repository. Trusted publishing / OIDC or documented org secrets are
  required.

## Out of scope

- Edge host adapters and cloud / multi-cloud placement (constitution v0.1
  non-goals; Edge remains pre-spec).
- npm `traverse-embedder-web` (Spec 048 / Decision 74).
- Spec 529 Certified-vs-Preview runner implementation.
- Changing Chicory, Wasmtime .NET, or wasmi engine selection except as
  ADR-0070 already selects wasmi 2.0.0.
- Re-publishing missing native artifacts onto an already-shipped tag.

## Implementation tickets

- Traverse #1370 — XCFramework rebuild (hardware-gated)
- Traverse #1371 — Maven Central
- Traverse #1372 — nuget.org
- reference-apps #308 — consume published coordinates
