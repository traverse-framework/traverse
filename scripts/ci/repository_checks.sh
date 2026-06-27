#!/usr/bin/env bash

set -euo pipefail

required_files=(
  "README.md"
  "LICENSE"
  "NOTICE"
  "CONTRIBUTING.md"
  "CODE_OF_CONDUCT.md"
  "SECURITY.md"
  "SUPPORT.md"
  ".editorconfig"
  ".specify/memory/constitution.md"
  "docs/quality-standards.md"
  "docs/compatibility-policy.md"
  "docs/v0.3.0-public-surface-compatibility.md"
  "docs/v0.3.0-source-build-consumer-packaging.md"
  "docs/v0.3.0-downstream-validation-path.md"
  "docs/youaskm3-v0.3.0-integration-readiness.md"
  "docs/releases/v0.5.0.md"
  "docs/troubleshooting.md"
  "docs/adapter-boundaries.md"
  "docs/contract-publication-policy.md"
  "docs/expedition-example-authoring.md"
  "docs/expedition-example-smoke.md"
  "docs/mcp-consumption-validation.md"
  "docs/mcp-stdio-server.md"
  "docs/youaskm3-canonical-mcp-client-path.md"
  "docs/mcp-real-agent-exercise.md"
  "docs/app-consumable-release-checklist.md"
  "docs/youaskm3-canonical-app-http-path.md"
  "docs/app-consumable-consumer-bundle.md"
  "docs/app-consumable-release-artifact.md"
  "docs/app-consumable-package-release-pointer.md"
  "docs/packaged-traverse-mcp-server-artifact.md"
  "docs/packaged-traverse-runtime-artifact.md"
  "docs/youaskm3-integration-validation.md"
  "docs/youaskm3-published-artifact-validation.md"
  "docs/youaskm3-compatibility-conformance-suite.md"
  "docs/youaskm3-real-shell-validation.md"
  "docs/downstream-app-mvp-conformance.md"
  "docs/wasm-agent-authoring-guide.md"
  "docs/wasm-microservice-authoring-guide.md"
  "specs/023-downstream-publication-strategy/spec.md"
  "specs/023-downstream-publication-strategy/data-model.md"
  "specs/023-downstream-publication-strategy/checklists/requirements.md"
  "apps/browser-consumer/README.md"
  "apps/browser-consumer/package.json"
  "apps/youaskm3-starter-kit/README.md"
  "apps/youaskm3-starter-kit/package.json"
  "docs/wasm-agent-team-readiness-example.md"
  "docs/app-consumable-acceptance.md"
  "docs/app-consumable-entry-path.md"
  "docs/executable-package-template.md"
  "docs/local-runtime-home.md"
  "docs/getting-started.md"
  "quickstart.md"
  "scripts/validate-setup.sh"
  "examples/expedition/runtime-requests/plan-expedition.json"
  "examples/hello-world/README.md"
  "examples/hello-world/runtime-requests/say-hello.json"
  "examples/hello-world/say-hello-agent/manifest.json"
  "examples/hello-world/say-hello-agent/build-fixture.sh"
  "contracts/examples/hello-world/capabilities/say-hello/contract.json"
  "workflows/examples/hello-world/say-hello/workflow.json"
  "docs/exception-process.md"
  "docs/project-management.md"
  "docs/multi-thread-workflow.md"
  "docs/zero-to-hero.md"
  "docs/ticket-standard.md"
  "docs/planning-board.md"
  "docs/ai-review-process.md"
  "docs/adr/README.md"
  "docs/adr/0001-rust-wasm-foundation.md"
  "scripts/ci/browser_adapter_smoke.sh"
  "apps/react-demo/server.mjs"
  "apps/react-demo/src/browser-adapter-client.js"
  "scripts/ci/react_demo_live_adapter_smoke.sh"
  "scripts/ci/mcp_stdio_server_execution_report_smoke.sh"
  "scripts/ci/youaskm3_compatibility_conformance.sh"
  "scripts/ci/downstream_app_mvp_conformance.sh"
  "scripts/ci/downstream_app_bundle_registration_smoke.sh"
  "scripts/ci/downstream_public_app_registration_smoke.sh"
  "scripts/ci/downstream_wasm_workflow_smoke.sh"
  "scripts/ci/downstream_model_dependency_smoke.sh"
  "scripts/ci/downstream_http_json_smoke.sh"
  "scripts/ci/downstream_mcp_smoke.sh"
  "scripts/ci/browser_consumer_package_smoke.sh"
  "scripts/ci/youaskm3_integration_validation.sh"
  "scripts/ci/youaskm3_published_artifact_validation.sh"
  "scripts/ci/youaskm3_real_shell_validation.sh"
  "scripts/ci/app_consumable_release_prep.sh"
  "scripts/ci/app_consumable_package_release_pointer.sh"
  "scripts/ci/wasm_agent_authoring_guide_smoke.sh"
  "scripts/ci/hello_world_example_smoke.sh"
  "scripts/ci/wasi_host_abi_imports.sh"
  "scripts/ci/zero_to_hero_acceptance.sh"
  "scripts/ci/wasm_microservice_authoring_guide_smoke.sh"
  "scripts/ci/downstream_publication_strategy_smoke.sh"
  "scripts/ci/mcp_stdio_server_smoke.sh"
  "scripts/ci/packaged_traverse_mcp_server_artifact_smoke.sh"
  "scripts/ci/packaged_traverse_runtime_artifact_smoke.sh"
  "scripts/ci/mcp_stdio_server_discovery_smoke.sh"
  "scripts/ci/mcp_stdio_server_execution_report_smoke.sh"
  "scripts/ci/mcp_real_agent_exercise_smoke.sh"
  "scripts/ci/project_board_audit.sh"
  "scripts/scaffold/hello_world_agent_scaffold.sh"
  "scripts/scaffold/new-capability.sh"
  "scripts/ci/new_capability_scaffold_smoke.sh"
  ".github/ISSUE_TEMPLATE/task.yml"
  "specs/001-foundation-v0-1/spec.md"
  "specs/001-foundation-v0-1/plan.md"
  "specs/001-foundation-v0-1/research.md"
  "specs/001-foundation-v0-1/data-model.md"
  "specs/004-spec-alignment-gate/spec.md"
  "specs/004-spec-alignment-gate/data-model.md"
  "specs/governance/approved-specs.json"
  "specs/022-mcp-wasm-server/spec.md"
  "specs/022-mcp-wasm-server/checklists/requirements.md"
  "specs/033-http-json-api/spec.md"
  "specs/033-http-json-api/openapi.yaml"
  "specs/033-http-json-api/context.md"
  "specs/034-programmatic-registration/spec.md"
  "specs/034-programmatic-registration/context.md"
  "specs/035-multi-agent-isolation/spec.md"
  "specs/035-multi-agent-isolation/context.md"
  "specs/README.md"
  "scripts/ci/openapi_structural_validation.sh"
  "scripts/ci/spec_context_check.sh"
)

for file in "${required_files[@]}"; do
  test -f "$file"
  test -s "$file"
done

if command -v rg >/dev/null 2>&1; then
  if rg -n "Cogollo|Cogolo" . \
    --hidden \
    -g '!.git' \
    -g '!.claude' \
    -g '!external' \
    -g '!references' \
    -g '!scripts/ci/repository_checks.sh'; then
    echo "Found stale project name references; expected 'Traverse'." >&2
    exit 1
  fi
else
  if grep -RInE \
    --exclude='repository_checks.sh' \
    --exclude-dir='.git' \
    --exclude-dir='.claude' \
    --exclude-dir='external' \
    --exclude-dir='references' \
    'Cogollo|Cogolo' .; then
    echo "Found stale project name references; expected 'Traverse'." >&2
    exit 1
  fi
fi

grep -q "GitHub Project 1" README.md
grep -q "Apache-2.0" README.md
grep -q "docs/adapter-boundaries.md" README.md
grep -q "docs/troubleshooting.md" README.md
grep -q "docs/getting-started.md" README.md
grep -q "quickstart.md" README.md
grep -q "bash scripts/validate-setup.sh" docs/getting-started.md
grep -q "bash scripts/validate-setup.sh" quickstart.md
grep -q "Definition of Done" docs/ticket-standard.md
grep -q "in-progress" docs/ticket-standard.md
grep -q "active branch, PR, or an explicitly assigned developer" docs/ticket-standard.md
grep -q "Validation" docs/ticket-standard.md
grep -q "future" docs/project-management.md
grep -q "in-progress" docs/project-management.md
! grep -q '^- `ready`$' docs/project-management.md
grep -q 'Potential parallel candidates should stay `Ready`' docs/project-management.md
grep -q "Project 1 status is the only actionability signal" docs/project-management.md
grep -q "project_board_audit.sh" docs/project-management.md
grep -q "Note" docs/project-management.md
grep -q "separate Codex threads" docs/project-management.md
grep -q "Blocked" docs/planning-board.md
grep -q "In Progress" docs/planning-board.md
grep -q "Only tickets with real active execution" docs/planning-board.md
grep -q "Note" docs/ticket-standard.md
! grep -q '^- `ready`$' docs/ticket-standard.md
grep -q "Use Project 1 status for availability" docs/ticket-standard.md
grep -q "One Codex thread is one active worker" docs/multi-thread-workflow.md
grep -q "Starter Prompts" docs/multi-thread-workflow.md
grep -q "project_board_audit.sh" docs/multi-thread-workflow.md
grep -q "bash scripts/ci/expedition_artifact_smoke.sh" docs/expedition-example-smoke.md
grep -q "bash scripts/ci/expedition_execution_smoke.sh" docs/expedition-example-smoke.md
grep -q "bash scripts/ci/expedition_trace_smoke.sh" docs/expedition-example-smoke.md
grep -q "bash scripts/ci/expedition_golden_path.sh" docs/expedition-example-smoke.md
grep -q "bash scripts/ci/browser_adapter_smoke.sh" docs/expedition-example-smoke.md
grep -q "bash scripts/ci/event_driven_workflow_smoke.sh" docs/expedition-example-smoke.md
grep -q "TRAVERSE_REPO_ROOT" docs/expedition-example-smoke.md
grep -q "bash scripts/ci/mcp_consumption_validation.sh" docs/mcp-consumption-validation.md
grep -q "docs/mcp-stdio-server.md" docs/mcp-consumption-validation.md
grep -q "docs/mcp-real-agent-exercise.md" docs/mcp-consumption-validation.md
grep -q "list_content_groups" docs/mcp-stdio-server.md
grep -q "describe_content_group" docs/mcp-stdio-server.md
grep -q "core-runtime-example" docs/mcp-stdio-server.md
grep -q "mcp_stdio_server_execution_report_smoke.sh" docs/mcp-stdio-server.md
grep -q "render_execution_report" docs/mcp-stdio-server.md
grep -q "docs/youaskm3-integration-validation.md" docs/mcp-consumption-validation.md
grep -q "docs/mcp-real-agent-exercise.md" README.md
grep -q "docs/youaskm3-compatibility-conformance-suite.md" README.md
grep -q "docs/youaskm3-real-shell-validation.md" README.md
grep -q "docs/youaskm3-published-artifact-validation.md" README.md
grep -q "docs/youaskm3-compatibility-conformance-suite.md" docs/app-consumable-entry-path.md
grep -q "docs/downstream-app-mvp-conformance.md" docs/app-consumable-entry-path.md
grep -q "docs/youaskm3-real-shell-validation.md" docs/app-consumable-entry-path.md
grep -q "docs/youaskm3-published-artifact-validation.md" docs/app-consumable-entry-path.md
grep -q "docs/youaskm3-compatibility-conformance-suite.md" docs/mcp-consumption-validation.md
grep -q "docs/youaskm3-compatibility-conformance-suite.md" docs/youaskm3-integration-validation.md
grep -q "docs/youaskm3-real-shell-validation.md" docs/youaskm3-integration-validation.md
grep -q "docs/youaskm3-published-artifact-validation.md" docs/youaskm3-integration-validation.md
grep -q "docs/downstream-app-mvp-conformance.md" docs/youaskm3-integration-validation.md
grep -q "traverse-cli app validate --manifest <path> --json" docs/cli-reference.md
grep -q "traverse-cli app register --manifest <path> --workspace <workspace-id> --json" docs/cli-reference.md
grep -q "status: already_registered" docs/cli-reference.md
grep -q "not an HTTP app registration endpoint" docs/cli-reference.md
grep -q "traverse-cli app validate --manifest <path> --json" docs/youaskm3-canonical-app-http-path.md
grep -q "traverse-cli app register --manifest <path> --workspace <workspace-id> --json" docs/youaskm3-canonical-app-http-path.md
grep -q "eventing-oriented contracts" docs/youaskm3-canonical-app-http-path.md
grep -q "downstream_public_app_registration_smoke.sh" docs/youaskm3-integration-validation.md
grep -q "docs/mcp-real-agent-exercise.md" docs/youaskm3-integration-validation.md
grep -q "youaskm3 compatibility conformance suite" docs/youaskm3-compatibility-conformance-suite.md
grep -q "version pairing" docs/youaskm3-compatibility-conformance-suite.md
grep -q "bash scripts/ci/youaskm3_compatibility_conformance.sh" docs/youaskm3-compatibility-conformance-suite.md
grep -q "bash scripts/ci/youaskm3_compatibility_conformance.sh" docs/youaskm3-integration-validation.md
grep -q "bash scripts/ci/youaskm3_published_artifact_validation.sh" docs/youaskm3-integration-validation.md
grep -q "the same Traverse v0.1 release pairing" docs/youaskm3-integration-validation.md
grep -q "youaskm3 real shell validation" docs/youaskm3-real-shell-validation.md
grep -q "openspec/specs/pwa-shell/spec.md" docs/youaskm3-real-shell-validation.md
grep -q "bash scripts/ci/youaskm3_real_shell_validation.sh" docs/youaskm3-real-shell-validation.md
grep -q "bash scripts/ci/mcp_real_agent_exercise_smoke.sh" docs/mcp-real-agent-exercise.md
grep -q "discover_capabilities" docs/mcp-real-agent-exercise.md
grep -q "discover_events" docs/mcp-real-agent-exercise.md
grep -q "discover_workflows" docs/mcp-real-agent-exercise.md
grep -q "execute_entrypoint" docs/mcp-real-agent-exercise.md
grep -q "render_execution_report" docs/mcp-real-agent-exercise.md

bash scripts/ci/openapi_structural_validation.sh
bash scripts/ci/spec_context_check.sh
grep -q "scripts/smoke.sh" docs/youaskm3-real-shell-validation.md
grep -q "apps/browser-consumer/README.md" docs/mcp-consumption-validation.md
grep -q "docs/app-consumable-consumer-bundle.md" README.md
grep -q "docs/app-consumable-package-release-pointer.md" README.md
grep -q "docs/packaged-traverse-mcp-server-artifact.md" README.md
grep -q "docs/wasm-agent-authoring-guide.md" README.md
grep -q "docs/wasm-microservice-authoring-guide.md" README.md
grep -q "docs/packaged-traverse-runtime-artifact.md" README.md
grep -q "specs/023-downstream-publication-strategy/spec.md" docs/app-consumable-release-artifact.md
grep -q "specs/023-downstream-publication-strategy/spec.md" docs/app-consumable-consumer-bundle.md
grep -q "docs/app-consumable-consumer-bundle.md" docs/app-consumable-entry-path.md
grep -q "docs/app-consumable-package-release-pointer.md" docs/app-consumable-entry-path.md
grep -q "docs/youaskm3-published-artifact-validation.md" docs/app-consumable-release-checklist.md
grep -q "versioned Traverse consumer bundle" docs/app-consumable-consumer-bundle.md
grep -q "supported version selection" docs/app-consumable-consumer-bundle.md
grep -q "installation steps" docs/app-consumable-consumer-bundle.md
grep -q "apps/browser-consumer/README.md" docs/app-consumable-consumer-bundle.md
grep -q "docs/mcp-stdio-server.md" docs/app-consumable-consumer-bundle.md
grep -q "docs/packaged-traverse-runtime-artifact.md" docs/app-consumable-consumer-bundle.md
grep -q "bash scripts/ci/app_consumable_release_prep.sh" docs/app-consumable-consumer-bundle.md
grep -q "docs/packaged-traverse-mcp-server-artifact.md" docs/mcp-stdio-server.md
grep -q "docs/packaged-traverse-mcp-server-artifact.md" docs/mcp-consumption-validation.md
grep -q "app-consumable v0.1" docs/app-consumable-release-checklist.md
grep -q "Release Blockers" docs/app-consumable-release-checklist.md
grep -q "Post-Release Follow-Up" docs/app-consumable-release-checklist.md
grep -q "quickstart.md" docs/app-consumable-release-checklist.md
grep -q "docs/app-consumable-consumer-bundle.md" docs/app-consumable-release-checklist.md
grep -q "docs/app-consumable-package-release-pointer.md" docs/app-consumable-release-checklist.md
grep -q "scripts/ci/youaskm3_published_artifact_validation.sh" docs/app-consumable-release-checklist.md
grep -q "publication bundle" docs/app-consumable-release-artifact.md
grep -q "docs/app-consumable-package-release-pointer.md" docs/app-consumable-release-artifact.md
grep -q "versioned consumer bundle" docs/app-consumable-release-artifact.md
grep -q "GitHub release entry" docs/app-consumable-release-artifact.md
grep -q "supported runnable artifact" docs/app-consumable-release-artifact.md
grep -q "docs/packaged-traverse-runtime-artifact.md" docs/app-consumable-release-artifact.md
grep -q "docs/packaged-traverse-mcp-server-artifact.md" docs/app-consumable-release-artifact.md
grep -q "docs/youaskm3-published-artifact-validation.md" docs/app-consumable-release-artifact.md
grep -q "bash scripts/ci/app_consumable_release_prep.sh" docs/app-consumable-release-artifact.md
grep -q "bash scripts/ci/wasm_agent_team_readiness_smoke.sh" docs/wasm-agent-team-readiness-example.md
grep -q "docs/wasm-agent-authoring-guide.md" docs/expedition-example-authoring.md
grep -q "docs/wasm-microservice-authoring-guide.md" docs/expedition-example-authoring.md
grep -q "bash scripts/ci/app_consumable_acceptance.sh" docs/app-consumable-acceptance.md
grep -q "React browser demo" docs/app-consumable-acceptance.md
grep -q "Canonical Rule" docs/app-consumable-entry-path.md
grep -q "Start Here" docs/app-consumable-entry-path.md
grep -q "quickstart.md" docs/app-consumable-entry-path.md
grep -q "bash scripts/ci/executable_package_template_smoke.sh" docs/executable-package-template.md
grep -q "docs/local-runtime-home.md" docs/executable-package-template.md
grep -q "cargo run -p traverse-cli -- bundle inspect examples/expedition/registry-bundle/manifest.json" docs/expedition-example-authoring.md
grep -q "cargo run -p traverse-cli -- expedition execute examples/expedition/runtime-requests/plan-expedition.json" docs/expedition-example-authoring.md
grep -q "cargo run -p traverse-cli -- trace inspect" docs/expedition-example-authoring.md
grep -q "cargo run -p traverse-cli -- bundle register examples/expedition/registry-bundle/manifest.json" docs/expedition-example-authoring.md
grep -q "workflows/examples/expedition/plan-expedition/workflow.json" docs/expedition-example-authoring.md
grep -q ".traverse/local/" docs/expedition-example-authoring.md
grep -q "capture-expedition-objective/contract.json" docs/getting-started.md
grep -q "docs/troubleshooting.md" docs/getting-started.md
grep -q "examples/hello-world/README.md" docs/getting-started.md
grep -q "cargo run -p traverse-cli -- bundle register" docs/getting-started.md
grep -q "cargo run -p traverse-cli -- expedition execute" docs/getting-started.md
grep -q "cargo run -p traverse-cli -- trace inspect" docs/getting-started.md
grep -q "bash scripts/ci/expedition_golden_path.sh" docs/getting-started.md
grep -q "docs/troubleshooting.md" quickstart.md
grep -q "docs/troubleshooting.md" docs/tutorial-index.md
grep -q "Repository Checks" docs/troubleshooting.md
grep -q "Rust Checks" docs/troubleshooting.md
grep -q "Coverage Gate" docs/troubleshooting.md
grep -q "Spec Alignment" docs/troubleshooting.md
grep -q "Generated Local State And Safe Cleanup" docs/troubleshooting.md
grep -q "cargo run -p traverse-cli -- agent execute" examples/hello-world/README.md
grep -q "hello.world.say-hello" examples/hello-world/README.md
grep -q "bash scripts/ci/runtime_home_smoke.sh" docs/local-runtime-home.md
grep -q "label: Definition of done" .github/ISSUE_TEMPLATE/task.yml
grep -q "label: Validation" .github/ISSUE_TEMPLATE/task.yml
grep -q "Specs Are Versioned, Immutable, and Merge-Gating" .specify/memory/constitution.md
grep -q "Non-Functional Requirements" .specify/memory/constitution.md
grep -q "Enterprise Quality Standards" .specify/memory/constitution.md
grep -q "Non-Functional Requirements" specs/001-foundation-v0-1/spec.md
grep -q "Non-Negotiable Quality Standards" specs/001-foundation-v0-1/spec.md
grep -q "AI Review Process" docs/ai-review-process.md
grep -q '"schema_version": "1.0.0"' specs/governance/approved-specs.json
grep -q "Spec-alignment gate implementation" docs/quality-standards.md
grep -q "docs/adapter-boundaries.md" docs/compatibility-policy.md
grep -q "specs/013-browser-runtime-subscription/spec.md" docs/adapter-boundaries.md
grep -q "specs/014-mcp-surface/spec.md" docs/adapter-boundaries.md
grep -q "mandatory sidecar topology" docs/adapter-boundaries.md
grep -q "optional adapter choices" docs/adapter-boundaries.md
grep -q "browser-adapter serve" apps/react-demo/README.md
grep -q "react_demo_live_adapter_smoke.sh" apps/react-demo/README.md
grep -q "same-origin local proxy" apps/react-demo/README.md
grep -q "app-consumable acceptance" apps/react-demo/README.md
grep -q "Run the local browser adapter proxy again" apps/react-demo/README.md
grep -q "browser-targeted consumer package" docs/app-consumable-consumer-bundle.md
grep -q "consumer bundle" docs/app-consumable-consumer-bundle.md
grep -q "docs/app-consumable-package-release-pointer.md" docs/app-consumable-consumer-bundle.md
grep -q "youaskm3 integration validation" README.md
grep -q "Traverse React demo serving on" apps/react-demo/server.mjs
grep -q "runLiveBrowserSubscription" apps/react-demo/src/browser-adapter-client.js
grep -q "applyBrowserSubscriptionMessage" apps/react-demo/src/browser-adapter-client.js
grep -q "App" apps/react-demo/src/main.js
grep -q "react_demo_live_adapter_smoke.sh" scripts/ci/react_demo_live_adapter_smoke.sh
grep -q "## Prerequisites" quickstart.md
grep -q "browser-adapter serve --bind 127.0.0.1:4174" quickstart.md
grep -q "node apps/react-demo/server.mjs --adapter http://127.0.0.1:4174 --port 4173" quickstart.md
grep -q "apps/browser-consumer/README.md" docs/app-consumable-entry-path.md
grep -q "git checkout v0.3.0" docs/v0.3.0-public-surface-compatibility.md
grep -q "docs/youaskm3-canonical-app-http-path.md" docs/v0.3.0-public-surface-compatibility.md
grep -q "docs/youaskm3-canonical-mcp-client-path.md" docs/v0.3.0-public-surface-compatibility.md
grep -q "Supply-chain evidence" docs/v0.3.0-public-surface-compatibility.md
grep -q "docs/v0.3.0-source-build-consumer-packaging.md" README.md
grep -q "docs/v0.3.0-downstream-validation-path.md" README.md
grep -q "docs/youaskm3-v0.3.0-integration-readiness.md" README.md
grep -q 'version = "0.5.0"' Cargo.toml
grep -q "docs/releases/v0.5.0.md" README.md
grep -q "Traverse v0.4.0" docs/releases/v0.4.0.md
grep -q "044-application-bundle-manifest" docs/releases/v0.4.0.md
grep -q "045-governed-model-dependency-resolution" docs/releases/v0.4.0.md
grep -q "bash scripts/ci/downstream_app_mvp_conformance.sh" docs/releases/v0.4.0.md
grep -q "traverse-sbom.cdx.json" docs/releases/v0.4.0.md
grep -q "Traverse v0.5.0" docs/releases/v0.5.0.md
grep -q "046-public-cli-app-registration" docs/releases/v0.5.0.md
grep -q "traverse-cli app validate --manifest <path> --json" docs/releases/v0.5.0.md
grep -q "traverse-cli app register --manifest <path> --workspace <workspace-id> --json" docs/releases/v0.5.0.md
grep -q "runtime loading from CLI-produced workspace app state" docs/releases/v0.5.0.md
grep -q "bash scripts/ci/downstream_app_mvp_conformance.sh" docs/releases/v0.5.0.md
grep -q "traverse-sbom.cdx.json" docs/releases/v0.5.0.md
grep -q "cargo build" docs/v0.3.0-source-build-consumer-packaging.md
grep -q "cargo run -p traverse-cli -- serve" docs/v0.3.0-source-build-consumer-packaging.md
grep -q "cargo run -p traverse-mcp -- stdio" docs/v0.3.0-source-build-consumer-packaging.md
grep -q "traverse-sbom.cdx.json" docs/v0.3.0-source-build-consumer-packaging.md
grep -q "No package-manager distribution" docs/v0.3.0-source-build-consumer-packaging.md
grep -q "Traverse v0.3.0 Downstream Validation Path" docs/v0.3.0-downstream-validation-path.md
grep -q "git checkout v0.3.0" docs/v0.3.0-downstream-validation-path.md
grep -q "bash scripts/ci/mcp_consumption_validation.sh" docs/v0.3.0-downstream-validation-path.md
grep -q "bash scripts/ci/app_consumable_acceptance.sh" docs/v0.3.0-downstream-validation-path.md
grep -q "bash scripts/ci/youaskm3_compatibility_conformance.sh" docs/v0.3.0-downstream-validation-path.md
grep -q "bash scripts/ci/repository_checks.sh" docs/v0.3.0-downstream-validation-path.md
grep -q "consumer_name: youaskm3" docs/v0.3.0-downstream-validation-path.md
grep -q "validated_flow_id: youaskm3_mcp_validation" docs/v0.3.0-downstream-validation-path.md
grep -q "docs/v0.3.0-downstream-validation-path.md" docs/v0.3.0-public-surface-compatibility.md
grep -q "docs/v0.3.0-downstream-validation-path.md" docs/v0.3.0-source-build-consumer-packaging.md
grep -q "docs/v0.3.0-downstream-validation-path.md" docs/youaskm3-canonical-app-http-path.md
grep -q "docs/v0.3.0-downstream-validation-path.md" docs/youaskm3-canonical-mcp-client-path.md
grep -q "docs/v0.3.0-downstream-validation-path.md" docs/youaskm3-integration-validation.md
grep -q "youaskm3 Traverse v0.3.0 Integration Readiness" docs/youaskm3-v0.3.0-integration-readiness.md
grep -q "Traverse \`v0.3.0\`" docs/youaskm3-v0.3.0-integration-readiness.md
grep -q "docs/youaskm3-canonical-mcp-client-path.md" docs/youaskm3-v0.3.0-integration-readiness.md
grep -q "docs/youaskm3-canonical-app-http-path.md" docs/youaskm3-v0.3.0-integration-readiness.md
grep -q "docs/v0.3.0-public-surface-compatibility.md" docs/youaskm3-v0.3.0-integration-readiness.md
grep -q "docs/v0.3.0-source-build-consumer-packaging.md" docs/youaskm3-v0.3.0-integration-readiness.md
grep -q "docs/v0.3.0-downstream-validation-path.md" docs/youaskm3-v0.3.0-integration-readiness.md
grep -q "Traverse Owns" docs/youaskm3-v0.3.0-integration-readiness.md
grep -q "youaskm3 Owns" docs/youaskm3-v0.3.0-integration-readiness.md
grep -q "First-Release Readiness Checklist" docs/youaskm3-v0.3.0-integration-readiness.md
grep -q "bash scripts/ci/youaskm3_compatibility_conformance.sh" docs/youaskm3-v0.3.0-integration-readiness.md
grep -q "bash scripts/ci/downstream_app_mvp_conformance.sh" docs/downstream-app-mvp-conformance.md
grep -q "TRAVERSE_RUN_LOCAL_OLLAMA_CONFORMANCE=1" docs/downstream-app-mvp-conformance.md
grep -q "044-application-bundle-manifest" docs/downstream-app-mvp-conformance.md
grep -q "045-governed-model-dependency-resolution" docs/downstream-app-mvp-conformance.md
grep -q "downstream app MVP conformance suite" scripts/ci/downstream_app_mvp_conformance.sh
grep -q "downstream_app_mvp_conformance.sh" docs/youaskm3-compatibility-conformance-suite.md
grep -q "docs/youaskm3-v0.3.0-integration-readiness.md" docs/youaskm3-integration-validation.md
grep -q "docs/youaskm3-v0.3.0-integration-readiness.md" docs/v0.3.0-public-surface-compatibility.md
grep -q "docs/v0.3.0-public-surface-compatibility.md" README.md
grep -q "docs/v0.3.0-public-surface-compatibility.md" docs/compatibility-policy.md
grep -q "docs/youaskm3-canonical-app-http-path.md" README.md
grep -q "docs/youaskm3-canonical-app-http-path.md" docs/app-consumable-entry-path.md
grep -q "Supported Traverse baseline: \`v0.3.0\`" docs/youaskm3-canonical-app-http-path.md
grep -q "cargo run -p traverse-cli -- serve" docs/youaskm3-canonical-app-http-path.md
grep -q ".traverse/server.json" docs/youaskm3-canonical-app-http-path.md
grep -q "POST /v1/workspaces/{workspace_id}/capabilities" docs/youaskm3-canonical-app-http-path.md
grep -q "POST /v1/workspaces/{workspace_id}/execute" docs/youaskm3-canonical-app-http-path.md
grep -q "GET /v1/workspaces/{workspace_id}/traces/{execution_id}" docs/youaskm3-canonical-app-http-path.md
grep -q "bash scripts/ci/app_consumable_acceptance.sh" docs/youaskm3-canonical-app-http-path.md
grep -q "browser-targeted consumer package" apps/browser-consumer/README.md
grep -q "browser_consumer_package_smoke.sh" apps/browser-consumer/README.md
grep -q "browser-hosted app" apps/browser-consumer/README.md
grep -q "bash scripts/ci/react_demo_live_adapter_smoke.sh" quickstart.md
grep -q "bash scripts/ci/react_demo_smoke.sh" quickstart.md
grep -q "## Known Limitations" quickstart.md
grep -q "bash scripts/ci/youaskm3_integration_validation.sh" docs/youaskm3-integration-validation.md
grep -q "cargo run -p traverse-mcp -- stdio" docs/mcp-stdio-server.md
grep -q "cargo run -p traverse-mcp -- stdio --simulate-startup-failure" docs/mcp-stdio-server.md
grep -q "bash scripts/ci/mcp_stdio_server_smoke.sh" docs/mcp-stdio-server.md
grep -q "bash scripts/ci/mcp_stdio_server_discovery_smoke.sh" docs/mcp-stdio-server.md
grep -q "bash scripts/ci/mcp_stdio_server_execution_report_smoke.sh" docs/mcp-stdio-server.md
grep -q "bash scripts/ci/mcp_stdio_server_discovery_smoke.sh" docs/mcp-stdio-server.md
grep -q "render_execution_report" docs/mcp-stdio-server.md
grep -q "list_entrypoints" docs/mcp-stdio-server.md
grep -q "describe_entrypoint" docs/mcp-stdio-server.md
grep -q "Supported Traverse baseline: \`v0.3.0\`" docs/youaskm3-canonical-mcp-client-path.md
grep -q "cargo run -p traverse-mcp -- stdio" docs/youaskm3-canonical-mcp-client-path.md
grep -q "stdio MCP client" docs/youaskm3-canonical-mcp-client-path.md
grep -q "bash scripts/ci/mcp_consumption_validation.sh" docs/youaskm3-canonical-mcp-client-path.md
grep -q "docs/youaskm3-canonical-mcp-client-path.md" README.md
grep -q "docs/youaskm3-canonical-mcp-client-path.md" docs/mcp-consumption-validation.md
grep -q "consumer_name: youaskm3" docs/youaskm3-integration-validation.md
grep -q "validated_flow_id: youaskm3_mcp_validation" docs/youaskm3-integration-validation.md
grep -q "bash scripts/ci/project_board_audit.sh" docs/project-management.md
grep -q "Open PR-backed tickets" docs/project-management.md
grep -q 'must be labeled `in-progress`' docs/multi-thread-workflow.md
grep -q "Dedicated Traverse MCP WASM Server Model" specs/022-mcp-wasm-server/spec.md
grep -q "Traverse runtime authority" specs/022-mcp-wasm-server/spec.md
grep -q "MCP transport concerns" specs/022-mcp-wasm-server/spec.md
grep -q "## Governing Spec" .github/pull_request_template.md

echo "Running new-capability scaffold smoke..."
TRAVERSE_REPO_ROOT="$(pwd)" bash "$(pwd)/scripts/ci/new_capability_scaffold_smoke.sh"

echo "Running WASI host ABI import whitelist verification..."
TRAVERSE_REPO_ROOT="$(pwd)" bash "$(pwd)/scripts/ci/wasi_host_abi_imports.sh"

echo "Repository checks passed."
