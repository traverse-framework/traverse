# Quality Standards

The shared, org-wide quality standards live in [`traverse-framework/.github`](https://github.com/traverse-framework/.github)'s `docs/quality-standards.md`. This repo has adopted **governance version 1.0.0**.

## What's Repo-Specific Here

Spec-alignment gate implementation is vendored locally (CI needs it in-repo to run):

- approved spec registry: `specs/governance/approved-specs.json`
- workflow job: `spec-alignment`
- script: `scripts/ci/spec_alignment_check.sh`

Coverage gate implementation, specific to this repo's crates:

- workflow job: `coverage-gate`
- script: `scripts/ci/coverage_gate.sh`
- protected crate list: `ci/coverage-targets.txt`

The coverage gate is merge-safe even before core logic exists. It passes when no protected crates are configured, and becomes enforcing as soon as core crates are added to `ci/coverage-targets.txt`.

## Nightly CI Gate

In addition to PR-gated checks, a nightly scheduled CI job runs the full golden-path acceptance suite independently of any PR activity.

**Schedule**: daily at 06:00 UTC (`.github/workflows/nightly.yml`)

**What it validates**:
- Zero-to-hero acceptance path (`scripts/ci/zero_to_hero_acceptance.sh`)
- Hello-world example smoke (`scripts/ci/hello_world_example_smoke.sh`)
- Expedition golden path (`scripts/ci/expedition_golden_path.sh`)
- Repository structure checks (`scripts/ci/repository_checks.sh`)
- Rust quality checks (fmt, clippy, tests)

**SLA**: any nightly failure must be investigated and resolved within 24 hours. A broken nightly that sits for more than 24 hours is a P1 issue.

**Manual trigger**: the workflow supports `workflow_dispatch` — trigger it from the GitHub Actions tab to validate a fix before the next scheduled run.

**Notification**: GitHub Actions sends an email to the repository owner on failure by default. No additional configuration required.
