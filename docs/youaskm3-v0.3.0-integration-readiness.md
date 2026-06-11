# youaskm3 Traverse v0.3.0 Integration Readiness

This page is the release-facing index for the first `youaskm3` release that depends on Traverse.

## Readiness Answer

Yes, `youaskm3` can honestly claim it is built on Traverse for runtime and MCP when it pins Traverse `v0.3.0` and follows the documented public surfaces below.

The honest claim is narrow:

> `youaskm3` uses Traverse `v0.3.0` for the source-build runtime surface, HTTP/JSON app integration path, and MCP stdio integration path. Product UI, ingestion behavior, and downstream release validation remain owned by `youaskm3`.

## Required Traverse Artifacts

| Readiness Area | Required Artifact |
|---|---|
| MCP client path | [docs/youaskm3-canonical-mcp-client-path.md](youaskm3-canonical-mcp-client-path.md) |
| HTTP/JSON app path | [docs/youaskm3-canonical-app-http-path.md](youaskm3-canonical-app-http-path.md) |
| Public compatibility statement | [docs/v0.3.0-public-surface-compatibility.md](v0.3.0-public-surface-compatibility.md) |
| Source-build packaging expectations | [docs/v0.3.0-source-build-consumer-packaging.md](v0.3.0-source-build-consumer-packaging.md) |
| Downstream validation evidence path | [docs/v0.3.0-downstream-validation-path.md](v0.3.0-downstream-validation-path.md) |

## Traverse Owns

Traverse owns these first-release responsibilities:

- the `v0.3.0` release tag and source-build baseline
- the documented `traverse-cli serve` HTTP/JSON app path
- the documented `cargo run -p traverse-mcp -- stdio` MCP path
- repository-local validation scripts for Traverse-side app and MCP evidence
- compatibility, packaging, and validation docs that downstream release notes can cite
- repository checks that keep the release-facing docs discoverable

## youaskm3 Owns

`youaskm3` owns these first-release responsibilities:

- product UI and user workflows
- ingestion, source presentation, and knowledge-app behavior
- app-specific MCP client wiring and runtime launch decisions
- downstream repo release gates and smoke tests
- any product claim beyond the documented Traverse `v0.3.0` runtime and MCP surfaces

## First-Release Readiness Checklist

Use this checklist before `youaskm3` cites Traverse in a release note or app-facing spec.

- [ ] `youaskm3` pins Traverse `v0.3.0` rather than `main` or an unpublished branch.
- [ ] The release notes cite the canonical MCP path.
- [ ] The release notes cite the canonical HTTP/JSON app path.
- [ ] The release notes cite the public surface compatibility statement.
- [ ] The release notes cite the source-build packaging expectations.
- [ ] The release notes cite the downstream validation path.
- [ ] The Traverse-side validation sequence passes from a clean `v0.3.0` checkout.
- [ ] Any `youaskm3` product claims are validated in the `youaskm3` repository, not inferred from Traverse checks.

## Traverse-Side Validation

From a clean Traverse checkout pinned to `v0.3.0`, use:

```bash
git clone https://github.com/enricopiovesan/Traverse.git
cd Traverse
git checkout v0.3.0
cargo build
bash scripts/ci/mcp_consumption_validation.sh
bash scripts/ci/app_consumable_acceptance.sh
bash scripts/ci/youaskm3_compatibility_conformance.sh
bash scripts/ci/repository_checks.sh
```

This is the same evidence path documented in [docs/v0.3.0-downstream-validation-path.md](v0.3.0-downstream-validation-path.md).

## Not Ready Claims

Do not claim that Traverse `v0.3.0` provides:

- a hosted production runtime
- native installers or package-manager distribution
- every possible MCP client integration
- `youaskm3` product completeness
- ingestion support such as `markitdown`
- a final `1.0` compatibility guarantee

Those claims require separate specs, tickets, implementation, and release evidence.
