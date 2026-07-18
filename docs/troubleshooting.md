# Traverse Troubleshooting Guide

Use this page when a local command or CI check fails and you need the shortest path back to a healthy repo state.

This guide is intentionally tied to the checks and docs that Traverse already ships today. It is not a generic Rust troubleshooting page.

## Start With The Failing Check

Match the failing command or CI job to the relevant section below:

| Failing Check | Typical Symptom | Go To |
|---|---|---|
| `bash scripts/ci/repository_checks.sh` | missing doc, stale link, missing expected string, stale project naming | [Repository Checks](#repository-checks) |
| `bash scripts/ci/rust_checks.sh` | `cargo fmt --check`, `cargo clippy`, or workspace test failure | [Rust Checks](#rust-checks) |
| `bash scripts/ci/coverage_gate.sh` | missing `cargo-llvm-cov`, coverage below threshold | [Coverage Gate](#coverage-gate) |
| `bash scripts/ci/spec_alignment_check.sh ...` or CI `spec-alignment` | PR body/spec mismatch, missing approved spec, invalid base SHA | [Spec Alignment](#spec-alignment) |
| `bash scripts/ci/react_demo_live_adapter_smoke.sh` | browser adapter flow does not start or complete | [Browser Adapter And Demo Smoke Paths](#browser-adapter-and-demo-smoke-paths) |
| `bash scripts/ci/mcp_consumption_validation.sh` or MCP stdio smoke paths | `traverse-mcp` stdio flow fails or documented MCP surface drifts | [MCP Validation And Stdio Server Paths](#mcp-validation-and-stdio-server-paths) |
| `bash scripts/ci/app_consumable_release_prep.sh` | release docs or bundle references are out of sync | [App-Consumable Release Prep](#app-consumable-release-prep) |

## Repository Checks

Command:

```bash
bash scripts/ci/repository_checks.sh
```

What this guard does:

- verifies required docs, scripts, and governed spec files exist
- checks for stale historical project naming drift
- checks that key docs still reference the required smoke paths and release docs

Common failure shapes:

- `test -f` or `test -s` fails for a required file
- `grep -q` fails because a doc no longer mentions a required command or linked document
- stale product naming appears in a changed file

What to check first:

1. Open the exact line in `scripts/ci/repository_checks.sh` that failed.
2. Confirm whether the failure is:
   - a missing file
   - a broken doc link
   - missing required wording in a doc
- stale historical project naming
3. Update the checked file instead of weakening the repository check unless the check is truly obsolete.

Useful follow-up commands:

```bash
sed -n '1,260p' scripts/ci/repository_checks.sh
rg -n "Traverse" README.md docs/
```

## Rust Checks

Command:

```bash
bash scripts/ci/rust_checks.sh
```

This script runs:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

### `cargo fmt --check` fails

Symptom:

- diff-style formatting output from `rustfmt`

Fix:

```bash
cargo fmt --all
```

Then rerun:

```bash
bash scripts/ci/rust_checks.sh
```

### `cargo clippy` fails

Symptom:

- warning promoted to error
- lint failure in one crate while formatting and tests are otherwise fine

Fix approach:

- address the lint directly in code
- avoid adding broad `allow` attributes unless there is a reviewed reason

Helpful command:

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

### `cargo test --workspace` fails

Symptom:

- one or more test cases fail
- snapshot, trace, or CLI summary expectations drift from current behavior

Fix approach:

- read the first failing test, not the whole tail of the output
- confirm whether the failure is:
  - a real behavior regression
  - an outdated expectation in a test
  - a doc-driven smoke assumption that changed

Helpful command:

```bash
cargo test --workspace
```

## Coverage Gate

Command:

```bash
bash scripts/ci/coverage_gate.sh
```

Common failure shapes:

- `cargo-llvm-cov is required for the coverage gate.`
- coverage for a protected crate drops below the required threshold

### `cargo-llvm-cov` is missing

Install it with:

```bash
cargo install cargo-llvm-cov
```

Then rerun:

```bash
bash scripts/ci/coverage_gate.sh
```

### Coverage drops below threshold

What to do:

1. Read the failing crate name from the script output.
2. Add or extend tests for the changed behavior.
3. Rerun the gate until the protected crate returns to threshold.

Helpful commands:

```bash
cat ci/coverage-targets.txt
bash scripts/ci/coverage_gate.sh
```

## Spec Alignment

Local command shape:

```bash
BASE_SHA=$(git merge-base origin/main HEAD) \
HEAD_SHA=HEAD \
bash scripts/ci/spec_alignment_check.sh /tmp/pr-body.md
```

Common failure shapes:

- PR body is missing `## Governing Spec`
- declared spec ids are not approved
- `BASE_SHA` is missing or not available locally
- changed files do not align with declared governing specs

What to check first:

1. Make sure the PR body has a `## Governing Spec` section.
2. Make sure every declared spec id exists in `specs/governance/approved-specs.json`.
3. Make sure `BASE_SHA` points to a commit that exists locally.
4. Check whether your changed paths are actually governed by the declared specs.

Helpful commands:

```bash
sed -n '1,220p' scripts/ci/spec_alignment_check.sh
git merge-base origin/main HEAD
sed -n '1,220p' specs/governance/approved-specs.json
```

## Browser Adapter And Demo Smoke Paths

Primary commands:

```bash
bash scripts/ci/browser_adapter_smoke.sh
bash scripts/ci/react_demo_live_adapter_smoke.sh
```

Common failure shapes:

- the browser adapter does not start
- the React demo cannot reach the adapter
- the live path never reaches terminal completion

What to check first:

1. Confirm the adapter command still works:

```bash
cargo run -p traverse-cli-rs -- browser-adapter serve --bind 127.0.0.1:4174
```

2. Confirm the React demo command still works:

```bash
node https://github.com/traverse-framework/App-References/tree/main/apps/react-demo/server.mjs --adapter http://127.0.0.1:4174 --port 4173
```

3. Re-read:
   - [quickstart.md](../quickstart.md)
   - [docs/app-consumable-entry-path.md](app-consumable-entry-path.md)

If local generated runtime state is suspicious or stale, inspect the runtime-owned workspace:

- [docs/local-runtime-home.md](local-runtime-home.md)

Treat `.traverse/local/` as runtime-owned generated state, not governed source.

## MCP Validation And Stdio Server Paths

Primary commands:

```bash
bash scripts/ci/mcp_consumption_validation.sh
bash scripts/ci/mcp_stdio_server_smoke.sh
bash scripts/ci/mcp_stdio_server_discovery_smoke.sh
bash scripts/ci/mcp_stdio_server_execution_report_smoke.sh
```

Common failure shapes:

- `traverse-mcp` stdio server does not boot
- documented MCP entry points drift from the real implementation
- downstream MCP validation no longer matches the published docs

What to check first:

1. Confirm the documented stdio command still works:

```bash
cargo run -p traverse-mcp -- stdio
```

2. Confirm the documented failure mode still works:

```bash
cargo run -p traverse-mcp -- stdio --simulate-startup-failure
```

3. Re-read:
   - [docs/mcp-stdio-server.md](mcp-stdio-server.md)
   - [docs/mcp-consumption-validation.md](mcp-consumption-validation.md)
   - [docs/mcp-real-agent-exercise.md](mcp-real-agent-exercise.md)

If the code and docs disagree, update the docs and validation together instead of fixing only one side.

## App-Consumable Release Prep

Primary commands:

```bash
bash scripts/ci/app_consumable_release_prep.sh
bash scripts/ci/app_consumable_package_release_pointer.sh
```

Common failure shapes:

- release artifact docs no longer point at the current bundle
- package pointer doc and release artifact doc drift apart
- consumer bundle docs stop matching the published validation path

What to check first:

1. Re-read the release-facing docs together:
   - [docs/app-consumable-release-artifact.md](app-consumable-release-artifact.md)
   - [docs/app-consumable-consumer-bundle.md](app-consumable-consumer-bundle.md)
   - [docs/app-consumable-package-release-pointer.md](app-consumable-package-release-pointer.md)
   - [docs/app-consumable-release-checklist.md](app-consumable-release-checklist.md)
2. Make sure the docs still point at the packaged runtime and MCP artifacts.
3. Make sure the linked validation scripts are still the ones the repo actually uses.

## Generated Local State And Safe Cleanup

When local runs behave strangely, first separate source-of-truth files from generated runtime state.

Source-of-truth files live in checked-in paths like:

- `contracts/`
- `workflows/`
- `examples/`
- `docs/`

Generated local runtime state lives under:

- `.traverse/local/`

Safe first steps:

```bash
ls -R .traverse/local 2>/dev/null || true
```

If you need to rebuild generated local state, prefer rerunning the documented command or smoke path before deleting anything.

Examples:

```bash
bash scripts/ci/browser_adapter_smoke.sh
bash scripts/ci/react_demo_live_adapter_smoke.sh
bash scripts/ci/mcp_stdio_server_smoke.sh
```

Use `cargo clean` only when you specifically need to clear compiled Rust build outputs:

```bash
cargo clean
```

## When You Still Do Not Know

Use this fallback order:

1. rerun the narrowest failing command directly
2. open the matching script in `scripts/ci/`
3. compare the script expectations with the relevant doc page
4. run `bash scripts/ci/repository_checks.sh`
5. fix the real drift instead of loosening the guardrail

## Validation

- `bash scripts/ci/repository_checks.sh`
- troubleshooting entries match current CI script names and documented runtime paths
