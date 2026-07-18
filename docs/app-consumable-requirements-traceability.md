# App-Consumable Requirements Traceability

This document maps the published Traverse v0.1 app-consumable state to the release-facing docs, validation paths, and the issue trail that now describes the shipped path.

Project 1 remains the coordination surface for the work, but this document is no longer a planning-time backlog view. It is the published-state trace map for the first app-consumable release set.

## Published v0.1 Traceability Map

| Requirement Area | Published Evidence | Traceability Note |
|---|---|---|
| Root app-consumable onboarding | [#122](https://github.com/traverse-framework/traverse/issues/122), [#127](https://github.com/traverse-framework/traverse/issues/127), [#142](https://github.com/traverse-framework/traverse/issues/142), [#143](https://github.com/traverse-framework/traverse/issues/143) | The first onboarding path is represented by the root docs, the release checklist, and the acceptance walkthrough. |
| Canonical docs entry path | [#144](https://github.com/traverse-framework/traverse/issues/144), [#267](https://github.com/traverse-framework/traverse/issues/267) | The README and quickstart now provide the canonical starting path instead of a planning-only entrypoint. |
| Release checklist and release-readiness evidence | [#127](https://github.com/traverse-framework/traverse/issues/127), [#145](https://github.com/traverse-framework/traverse/issues/145), [#150](https://github.com/traverse-framework/traverse/issues/150) | The release checklist, the release artifact, and the release-prep evidence describe the published v0.1 bundle. |
| Versioned consumer bundle and installation steps | [#176](https://github.com/traverse-framework/traverse/issues/176) | The versioned consumer bundle is the supported downstream install surface for app consumers. |
| Live browser-consumer path | [#120](https://github.com/traverse-framework/traverse/issues/120), [#121](https://github.com/traverse-framework/traverse/issues/121), [#123](https://github.com/traverse-framework/traverse/issues/123) | The browser adapter and live demo path are the supported consumable path for the published release. |
| Downstream consumer contract and app-facing validation | [#126](https://github.com/traverse-framework/traverse/issues/126), [#128](https://github.com/traverse-framework/traverse/issues/128), [#129](https://github.com/traverse-framework/traverse/issues/129) | The consumer contract and validation path define what downstream apps may rely on. |
| Real browser-hosted `youaskm3` shell validation | [#179](https://github.com/traverse-framework/traverse/issues/179) | The real downstream shell validation proves the published bundle works in a browser-hosted consumer. |
| Published-artifact validation against packaged Traverse runtime and MCP artifacts | [#200](https://github.com/traverse-framework/traverse/issues/200), [docs/youaskm3-published-artifact-validation.md](youaskm3-published-artifact-validation.md) | This is the published-artifact proof path for the released runtime and MCP artifacts. |
| MCP WASM server model and validation | [#146](https://github.com/traverse-framework/traverse/issues/146), [#158](https://github.com/traverse-framework/traverse/issues/158), [#148](https://github.com/traverse-framework/traverse/issues/148) | The MCP surface is now part of the published v0.1 app-consumable release story. |

## Published Release Bundle

The published release picture is anchored by these docs:

- [docs/app-consumable-release-artifact.md](app-consumable-release-artifact.md)
- [docs/app-consumable-consumer-bundle.md](app-consumable-consumer-bundle.md)
- [docs/app-consumable-package-release-pointer.md](app-consumable-package-release-pointer.md)
- [docs/app-consumable-release-checklist.md](app-consumable-release-checklist.md)
- [docs/packaged-traverse-runtime-artifact.md](packaged-traverse-runtime-artifact.md)
- [docs/packaged-traverse-mcp-server-artifact.md](packaged-traverse-mcp-server-artifact.md)
- [docs/youaskm3-published-artifact-validation.md](youaskm3-published-artifact-validation.md)

The release artifact and publication bundle are the bridge between the runtime and MCP artifact docs, the consumer bundle, the quickstart, and the published-artifact validation path.

## Historical Planning Language

The old `needs-spec` and `open-first-release` wording is historical planning language from before the v0.1 release view was published. It is no longer the primary way this work is described.

The current published-state vocabulary uses release artifacts, consumer bundles, package pointers, and validation evidence instead of backlog-era planning labels.

## Deferred Follow-Up

The following items are still valid follow-up topics after the published v0.1 release set:

- broader packaging polish beyond the supported consumer path
- future host-model experiments
- broader performance baselines beyond the first app-consumable release needs
- future consumer templates and extra downstream apps

## Rule

If a new app-consumable requirement appears and cannot be mapped to one or more published trace items above, create the missing issue first and add it to Project 1 before calling the release backlog complete.
