# App-Consumable Documentation Entry Path

This is the canonical documentation path for humans and coding agents working on the first app-consumable Traverse flow.

## Start Here

1. Read the repository root [README.md](../README.md)
2. Open [quickstart.md](../quickstart.md)
3. Use the relevant deeper docs only after the quickstart path is clear:
   - [docs/cli-reference.md](cli-reference.md)
   - [docs/youaskm3-canonical-app-http-path.md](youaskm3-canonical-app-http-path.md)
   - [docs/app-consumable-acceptance.md](app-consumable-acceptance.md)
   - [docs/app-consumable-release-checklist.md](app-consumable-release-checklist.md)
   - [docs/app-consumable-consumer-bundle.md](app-consumable-consumer-bundle.md)
   - [docs/app-consumable-package-release-pointer.md](app-consumable-package-release-pointer.md)
   - [docs/app-consumable-requirements-traceability.md](app-consumable-requirements-traceability.md)
   - [docs/youaskm3-integration-validation.md](youaskm3-integration-validation.md)
   - [docs/downstream-app-mvp-conformance.md](downstream-app-mvp-conformance.md)
   - [docs/youaskm3-published-artifact-validation.md](youaskm3-published-artifact-validation.md)
   - [docs/youaskm3-compatibility-conformance-suite.md](youaskm3-compatibility-conformance-suite.md)
   - [docs/youaskm3-real-shell-validation.md](youaskm3-real-shell-validation.md)
   - [apps/browser-consumer/README.md](../apps/browser-consumer/README.md)

## Canonical Rule

If a new human or agent asks where to begin, point them to the README first and then to the root quickstart.

## Why This Exists

- The README is the front door.
- The quickstart is the first executable consumer path.
- The canonical `youaskm3` HTTP path explains how downstream apps consume `traverse-cli serve`, `.traverse/server.json`, registration, execution, and trace fetch on the released `v0.3.0` baseline.
- The CLI reference explains the supported command surface and separates public commands from internal/test-only paths.
- The versioned consumer bundle explains what a downstream app installs and which released surfaces it may rely on.
- The package release pointer explains how the governed app-consumable package release is identified downstream.
- The published-artifact validation explains how `youaskm3` consumes the released runtime and MCP artifacts.
- The conformance suite explains how the released Traverse and `youaskm3` surfaces are proven together.
- The downstream app MVP conformance path proves public CLI app validation, public CLI app registration, and runtime loading from durable workspace state.
- The real-shell validation explains how the browser-hosted `youaskm3` shell is checked against the released Traverse consumer artifacts.
- The deeper docs explain validation, release, and traceability after the first path is understood.
- Competing entrypoints should be treated as references, not as the first recommended path.

## Validation

- The README links to the root quickstart.
- The quickstart links to the deeper app-consumable docs when needed.
- The canonical path is easy to describe without repository archaeology.
