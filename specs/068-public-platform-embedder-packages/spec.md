# Feature Specification: Public Consumable Platform Embedder Packages

**Feature Branch**: `068-public-platform-embedder-packages`
**Created**: 2026-07-12
**Status**: Draft — external review required before approval
**Version**: 1.0.0-draft
**Input**: Traverse #645; the App Reference Phase 3 blocker audit. Spec 057
defines an embedder IDL and conformance shape, but its implementation ticket
#553 did not produce a package consumable by application clients.

## Purpose

Define what Traverse must ship for a downstream application to embed the
runtime in production. A consumable embedder package is more than an IDL,
schema validator, or CLI test fixture: it is a versioned public artifact with
documented APIs, supported lifecycle semantics, deterministic test seams, and
release evidence.

This successor specification makes Spec 057 executable for the initial
platform set: Web/TypeScript, Swift, Kotlin/Android, .NET/WinUI, and
Rust/GTK/CLI.

## Relationship to Existing Specifications

| Specification | Relationship |
| --- | --- |
| 057-embeddable-runtime-host | Defines the platform-neutral operations and thin-embedder boundary; this spec defines public package delivery for those operations. |
| 058-workflow-pipeline-execution | `submit` must accept the same workflow and capability identifiers. |
| 052-app-state-machine | State-machine execution remains runtime-owned. |
| 053-conditional-state-transitions | Conditional routing remains runtime-owned and is surfaced through events only. |
| 033-http-json-api | Development sidecar only; not a production requirement for a packaged embedder. |

## Functional Requirements

- **FR-001**: Each supported platform MUST provide a public, versioned package
  or crate that implements `embedder-api/1.0.0` operations: `init`,
  `shutdown`, `submit`, and `subscribe`.
- **FR-002**: A package MUST load an application-owned bundle containing the
  runtime WASM, app manifest, component manifests, and capability artifacts;
  production execution MUST NOT require `traverse-cli serve` or
  `.traverse/server.json` discovery.
- **FR-003**: `submit` MUST accept the workflow/capability identifiers and
  JSON inputs defined by the bundled manifest and MUST surface runtime errors
  as structured, versioned events or results.
- **FR-004**: `subscribe` MUST preserve ordering and payload semantics of the
  runtime event stream. The package MAY adapt idiomatic callback, async-stream,
  or observable mechanics, but MUST NOT alter runtime event meaning.
- **FR-005**: Packages MUST NOT implement business rules, workflow transitions,
  capability-output derivation, or client-side replicas of conditional routing.
- **FR-006**: Every package MUST expose a deterministic test double or
  in-memory harness that implements the same public boundary without replacing
  production business logic.
- **FR-007**: Public package documentation MUST state supported platforms,
  runtime-WASM compatibility, bundle input shape, shutdown/cancellation
  behavior, error mapping, and upgrade policy.
- **FR-008**: Package releases MUST use semantic versions and publish release
  evidence containing package version, embedded runtime-WASM digest, embedder
  API/conformance version, and supported host versions.
- **FR-009**: Every package MUST pass the shared embedder conformance corpus
  for its declared embedder API version before release.
- **FR-010**: A platform package is complete only when the relevant
  App-References integration executes a bundled workflow without a sidecar.

## Platform Delivery Matrix

| Platform | Public delivery | Downstream ticket |
| --- | --- | --- |
| Web/React | TypeScript package | reference-apps #113 |
| iOS/macOS | Swift Package | reference-apps #114 |
| Android Compose | Kotlin/Android artifact | reference-apps #115 |
| Windows WinUI | .NET package | reference-apps #116 |
| Linux GTK/CLI | Rust crate | reference-apps #117 |

## Non-Functional Requirements

- **NFR-001 Compatibility**: A package MUST reject an incompatible bundle or
  runtime version deterministically and explain the mismatch without silently
  falling back to a sidecar.
- **NFR-002 Traceability**: The release evidence MUST be sufficient to connect
  a downstream binary to its package version, runtime digest, and conformance
  result.
- **NFR-003 Portability**: Platform adaptation is permitted only at the host
  boundary; application capability and workflow semantics remain portable.
- **NFR-004 Security**: Packages MUST not expose secrets in errors, events,
  test evidence, or bundle metadata.

## Acceptance Scenarios

1. Given a Web/React app bundle, when the app invokes `submit` without a
   sidecar process, then it receives runtime-owned pipeline output through the
   public package.
2. Given an incompatible runtime-WASM/bundle pairing, when `init` is called,
   then the package returns a stable compatibility error and does not attempt
   network or sidecar fallback.
3. Given any two packages implementing `embedder-api/1.0.0`, when they run the
   shared conformance corpus, then they satisfy the same lifecycle, event, and
   structured-error expectations.

## Out of Scope

- New business capabilities or UI screens.
- Replacing the dev sidecar for development tooling.
- A single cross-language implementation technology or distribution registry.
- Platform-specific extensions to `embedder-api/1.0.0`.

## Implementation Tickets

- Traverse #646 — Web/TypeScript
- Traverse #647 — Swift
- Traverse #648 — Kotlin/Android
- Traverse #649 — .NET/WinUI
- Traverse #650 — Rust/GTK/CLI
