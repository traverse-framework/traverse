# Traverse v0.1 App-Consumable Release Checklist

This checklist is the release decision aid for the first app-consumable Traverse release.

It does not redefine the downstream contract or the validation specs. It only states what must be true before Traverse can claim `app-consumable v0.1`, and what can safely wait until after release.

## Governing Specs

- `specs/019-downstream-consumer-contract/spec.md`
- `specs/020-downstream-integration-validation/spec.md`
- `specs/021-app-facing-operational-constraints/spec.md`

## Release Blockers

Traverse MUST NOT claim `app-consumable v0.1` unless all of the following are satisfied:

- [ ] README.md, quickstart.md, and docs/what-can-i-build.md have been updated
  for this release following the product-writing principles in
  [.specify/memory/readme-principles.md](../.specify/memory/readme-principles.md).
  Run the `update-docs` skill in Claude Code to apply them.

- [ ] The governed browser consumer path exists and is documented in [quickstart.md](../quickstart.md).
- [ ] The live local browser adapter path passes [scripts/ci/react_demo_live_adapter_smoke.sh](../scripts/ci/react_demo_live_adapter_smoke.sh).
- [ ] The browser demo path is documented as a real live adapter consumer in [apps/react-demo/README.md](../apps/react-demo/README.md).
- [ ] The first versioned Traverse consumer bundle is documented in [docs/app-consumable-consumer-bundle.md](app-consumable-consumer-bundle.md).
- [ ] The downstream MCP consumption path exists and passes [scripts/ci/mcp_consumption_validation.sh](../scripts/ci/mcp_consumption_validation.sh).
- [ ] The first real `youaskm3` integration path exists and passes [scripts/ci/youaskm3_integration_validation.sh](../scripts/ci/youaskm3_integration_validation.sh).
- [ ] The real browser-hosted `youaskm3` shell validation exists and passes [scripts/ci/youaskm3_real_shell_validation.sh](../scripts/ci/youaskm3_real_shell_validation.sh).
- [ ] The published-artifact validation path exists and passes [scripts/ci/youaskm3_published_artifact_validation.sh](../scripts/ci/youaskm3_published_artifact_validation.sh).
- [ ] The published-artifact validation path is documented in [docs/youaskm3-published-artifact-validation.md](youaskm3-published-artifact-validation.md).
- [ ] The end-to-end acceptance path exists and passes [scripts/ci/app_consumable_acceptance.sh](../scripts/ci/app_consumable_acceptance.sh).
- [ ] The package release pointer exists and is documented in [docs/app-consumable-package-release-pointer.md](app-consumable-package-release-pointer.md).
- [ ] The operational constraints for app-facing browser and MCP surfaces are documented in [docs/adapter-boundaries.md](adapter-boundaries.md) and [docs/compatibility-policy.md](compatibility-policy.md).
- [ ] The consumer contract and integration-validation model remain aligned with approved governing specs.

If any item above is unchecked, `app-consumable v0.1` is blocked.

## Required Evidence

The release decision should be backed by:

- the release artifact and publication bundle definition
- the versioned consumer bundle definition
- the first app-consumable quickstart
- the browser live-adapter smoke path
- the MCP consumption validation path
- the first real `youaskm3` integration validation path
- the real browser-hosted `youaskm3` shell validation path
- the published-artifact validation path against the packaged runtime and MCP artifacts
- the published-artifact validation doc
- the package release pointer path
- the end-to-end app-consumable acceptance path
- reviewable PR checks on the release-related documentation and validation artifacts

## Post-Release Follow-Up

The following are valid follow-up items after `app-consumable v0.1` and do not block the first release:

- release automation and packaging polish
- broader app-consumer templates for future downstream apps
- stronger production deployment hardening
- full auth and multi-tenant policy work
- broader performance baselines and load testing
- additional downstream validation paths beyond `youaskm3`

## Reviewer Shortcut

A reviewer can answer the release question by checking:

1. the governed consumer contract
2. the downstream integration-validation spec
3. the operational-constraints spec
4. the quickstart
5. the versioned consumer bundle
6. the live browser adapter smoke path
7. the MCP validation path
8. the first real `youaskm3` integration validation path
9. the real browser-hosted `youaskm3` shell validation path
10. the published-artifact validation path
11. the published-artifact validation doc
12. the end-to-end acceptance path
13. the release artifact and publication bundle definition

If those artifacts and checks exist and are passing, the first app-consumable release can be evaluated on evidence rather than interpretation.
