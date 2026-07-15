# Traverse v0.1 App-Consumable Consumer Bundle

This document defines the first versioned Traverse consumer bundle for downstream app integration.

The bundle is the release-facing adoption package for downstream apps such as `youaskm3`. It tells a downstream team what to install, which Traverse release tag to pin, and which public surfaces are supported together.
The bundle is paired with the package release pointer at [docs/app-consumable-package-release-pointer.md](app-consumable-package-release-pointer.md).
The downstream publication strategy for packaged Traverse runtime and MCP artifacts is defined in [specs/023-downstream-publication-strategy/spec.md](../specs/023-downstream-publication-strategy/spec.md).
The packaged runtime artifact is defined in [docs/packaged-traverse-runtime-artifact.md](packaged-traverse-runtime-artifact.md).

## Bundle Purpose

The consumer bundle exists so a downstream app can adopt Traverse without depending on source checkout details, internal crate layout, or ad hoc environment knowledge.

It is a versioned compatibility record, not a new runtime behavior layer.

## Supported v0.1 Surfaces

For the first app-consumable release, the supported bundle points to:

- the browser-targeted consumer package at [https://github.com/traverse-framework/reference-apps/tree/main/apps/browser-consumer/README.md](../https://github.com/traverse-framework/reference-apps/tree/main/apps/browser-consumer/README.md)
- the dedicated MCP stdio server package at [docs/mcp-stdio-server.md](mcp-stdio-server.md)
- the live browser adapter path described in [quickstart.md](../quickstart.md)
- the downstream integration and validation docs for `youaskm3`

Those surfaces must be consumed as one versioned release set.

## Version Selection

This section defines the supported version selection rules for downstream apps.

Downstream apps SHOULD pin the same Traverse release tag for the consumer bundle, browser consumer package, and dedicated MCP surface references.

Downstream apps SHOULD NOT mix a released consumer bundle with undocumented `main`-branch internals.

If a downstream app needs a newer Traverse surface, it SHOULD move to a newer approved release bundle instead of cherry-picking unreleased pieces.

## Installation Steps

This section describes the installation steps for downstream repos.

1. Select the approved Traverse v0.1 release tag.
2. Follow the release publication bundle and quickstart references from the repo.
3. Consume the browser-targeted consumer package for the browser-hosted path.
4. Consume the dedicated MCP stdio server package for the MCP-facing path.
5. Run the documented validation commands before declaring the integration complete.

## Bundle Contents

The first versioned consumer bundle MUST include:

- the approved release tag or equivalent release pointer
- the browser-targeted consumer package reference
- the dedicated MCP stdio server package reference
- the app-consumable quickstart reference
- the browser live-adapter smoke reference
- the MCP consumption validation reference
- the `youaskm3` integration validation reference
- the app-consumable acceptance reference
- the release artifact and publication bundle reference
- the package release pointer reference

## Verification

A reviewer can verify the consumer bundle with:

```bash
bash scripts/ci/app_consumable_release_prep.sh
```

The release prep command confirms that the consumer bundle, release artifact, quickstart, and traceability docs are all present and linked together.

## Known Limits

The first versioned consumer bundle does not claim:

- full auth hardening
- multi-tenant guarantees
- remote deployment guarantees beyond the approved local and documented paths
- support for unreleased internal Traverse surfaces
