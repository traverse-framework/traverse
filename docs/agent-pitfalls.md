# Agent Pitfalls (Traverse)

Operational lessons paid for with failed CI. Read when the matching symptom
appears. Org-wide rules stay in `traverse-framework/.github`
(`docs/ai-agent-hardening.md`, `docs/antipatterns.md`, `docs/runbook.md`) —
this file is Traverse-only.

When a new failure costs a full CI round-trip, add an entry here (symptom,
cause, fix) in the same PR that hardens against it.

## Coverage gate locally

- **Symptom:** `cargo llvm-cov` / `coverage_gate.sh` fails locally with
  phantom uncovered lines, or disagrees with CI.
- **Cause:** Wrong LLVM tools, or stale `.profraw` / `.profdata` from an
  earlier run aggregated into the next.
- **Fix:** Use the same Rust toolchain CI pins (see `.github/workflows/ci.yml`).
  Run `cargo llvm-cov clean --workspace` before the gate. Prefer
  `bash scripts/ci/coverage_gate.sh`. If a single region stays "uncovered" on
  every host, restructure so the arm is tested or unreachable without UB —
  do not lower `ci/coverage-targets.txt` floors.

## First push and the pre-push hook

- **Symptom:** `git push` of a brand-new branch fails with "no PR exists".
- **Cause:** The hook used to require an open PR, which taught agents
  `--no-verify`.
- **Fix:** Write the PR body to a file and push with
  `TRAVERSE_PR_BODY=/path/to/body.md git push -u origin HEAD`
  (or place it at `.git/traverse-pr-body.md`). Documentation-only changes run
  only PR-body + spec-alignment. Never `--no-verify`.

## crates.io publishing tokens

- **Symptom:** Tag publish fails auth even though the org secret is set.
- **Cause:** A repo-level `CARGO_REGISTRY_TOKEN` overrides the org-level
  secret and can be stale.
- **Fix:** Prefer the org secret; remove or rotate a repo-level override
  before cutting a release tag.

## Project board GraphQL cost

- **Symptom:** `gh project item-list` fails with a query cost / complexity
  error.
- **Cause:** Large `--limit` without a `--query` filter.
- **Fix:** Keep `--limit` modest (≤100–300) and filter with `--query` /
  `--jq` (for example `status:Ready`). Page if needed.

## Spec-alignment and `docs/decision-log.md`

- **Symptom:** Spec-alignment fails with
  `coverage.spec_not_declared` on `docs/decision-log.md` or other mapped
  paths.
- **Cause:** The PR body did not declare a governing approved-spec id that
  maps to the changed files.
- **Fix:** Under `## Governing Spec`, list at least one mapped approved
  spec (backticked list items). If a decision amends no product spec, still
  declare the mapped gate specs for the files you touched (for example
  `004-spec-alignment-gate`, `048-semver-publishing-pipeline` for CI/changelog
  paths), or use `no-spec-needed` only when the file map allows it.

## Immutable specs

- **Symptom:** Implementation drifts from an approved spec and CI blocks.
- **Cause:** Specs are immutable once approved; code cannot silently diverge.
- **Fix:** Amend the governing spec in the **same** implementation PR (and
  update contracts/tests), or split a follow-up ticket — do not weaken gates.

## Decision log before reopening

- **Symptom:** A "blocked" ticket is reopened and re-litigated.
- **Cause:** The answer is already in `docs/decision-log.md`.
- **Fix:** Search the decision log by issue number first. If a decision
  exists, implement or close; do not restart the brainstorm.
