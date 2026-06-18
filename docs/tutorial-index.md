# Traverse Tutorial Index

This is the single entry point for new Traverse developers and coding agents.

## Choose Your Path

Pick the path that matches your goal before reading further:

| Goal | Start here |
|---|---|
| Register and invoke a capability end to end | [docs/getting-started.md](getting-started.md) |
| Consume Traverse from a browser app | [quickstart.md](../quickstart.md) |
| Follow the full ordered onboarding sequence | Continue below |

All three paths are documented in this repo. The descriptions above tell you which one to open first; the ordered sequence below links them all together when you are ready for more.

## Full Ordered Sequence

Use it in sequence unless you already know the slice you need:

1. [README.md](../README.md)
2. [docs/getting-started.md](getting-started.md)
3. [docs/capability-contract-authoring-guide.md](capability-contract-authoring-guide.md)
4. [docs/workflow-composition-guide.md](workflow-composition-guide.md)
5. [docs/event-publishing-tutorial.md](event-publishing-tutorial.md)
6. [quickstart.md](../quickstart.md)
7. [docs/app-consumable-entry-path.md](app-consumable-entry-path.md)
8. [docs/browser-adapter.md](browser-adapter.md)
9. [docs/expedition-example-authoring.md](expedition-example-authoring.md)
10. [docs/wasm-agent-authoring-guide.md](wasm-agent-authoring-guide.md)
11. [docs/wasm-microservice-authoring-guide.md](wasm-microservice-authoring-guide.md)
12. [docs/app-consumable-acceptance.md](app-consumable-acceptance.md)
13. [docs/app-consumable-release-checklist.md](app-consumable-release-checklist.md)
14. [docs/app-consumable-consumer-bundle.md](app-consumable-consumer-bundle.md)
15. [docs/youaskm3-integration-validation.md](youaskm3-integration-validation.md)
16. [docs/youaskm3-published-artifact-validation.md](youaskm3-published-artifact-validation.md)
17. [docs/youaskm3-compatibility-conformance-suite.md](youaskm3-compatibility-conformance-suite.md)
18. [docs/youaskm3-real-shell-validation.md](youaskm3-real-shell-validation.md)
19. [docs/quality-standards.md](quality-standards.md)
20. [docs/compatibility-policy.md](compatibility-policy.md)
21. [docs/multi-thread-workflow.md](multi-thread-workflow.md)
22. [docs/project-management.md](project-management.md)
23. [docs/troubleshooting.md](troubleshooting.md)
24. [docs/adr/README.md](adr/README.md)

## How To Read It

If your goal is only the first governed capability path, stop after `docs/getting-started.md`.

If your goal is the first app-consumable browser flow, continue through `quickstart.md` and `docs/app-consumable-entry-path.md`.

If your goal is downstream app support such as `youaskm3`, continue into the app-consumable and validation docs after that.

If your goal is documentation hygiene, onboarding, or process work, finish with the standards, compatibility, workflow, and project-management docs.

If any step fails while you are following this sequence, jump to [docs/troubleshooting.md](troubleshooting.md) before guessing.

## Reference Docs (Authoring and Architecture)

These docs are not part of the linear onboarding sequence but are essential references when authoring new capabilities, contracts, or agents:

- [docs/architecture-execution-models.md](architecture-execution-models.md) — WASM, MCP, and browser adapter: when to use each
- [docs/unified-architectural-blueprint.md](unified-architectural-blueprint.md) — cross-spec interfaces (skeleton)
- [docs/capability-contract-authoring-guide.md](capability-contract-authoring-guide.md) — full field reference, authoring steps, constraint and lifecycle tables
- [docs/event-contract-authoring-guide.md](event-contract-authoring-guide.md) — how to author an event contract from scratch
- [docs/workflow-contract-authoring-guide.md](workflow-contract-authoring-guide.md) — node/edge model, direct vs event edges, authoring steps
- [docs/registry-bundle-authoring-guide.md](registry-bundle-authoring-guide.md) — how to author a bundle manifest for a new domain
- [docs/wasm-agent-authoring-guide.md](wasm-agent-authoring-guide.md) — stub vs. real implementation, build-fixture.sh, model_dependencies
- [docs/model-dependency-authoring-guide.md](model-dependency-authoring-guide.md) — governed app manifest schema for real model candidates
