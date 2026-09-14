# Contributing

Thanks for contributing to Traverse.

## First contributions (start here)

You do **not** need a new governing spec for small, labeled first PRs.

1. Pick an open issue labeled [`good first issue`](https://github.com/traverse-framework/traverse/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22) or [`help wanted`](https://github.com/traverse-framework/traverse/labels/help%20wanted) — also check [`website`](https://github.com/traverse-framework/website/labels/help%20wanted), [`registry`](https://github.com/traverse-framework/registry/labels/help%20wanted), and [`claude-skills`](https://github.com/traverse-framework/claude-skills/labels/help%20wanted).
2. Comment on the issue so we know you are taking it.
3. Open a focused PR that links the issue. For docs/examples marked `no-spec-needed`, say that in the PR body and skip inventing a new `specs/` slice.

Good first shapes right now: MCP client config docs, TypeScript/Python consume examples, website polish, one small registry capability, skill-template wording.

Please still read [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md), [SECURITY.md](SECURITY.md), and accept the CLA at [`traverse-framework/.github/CLA.md`](https://github.com/traverse-framework/.github/blob/main/CLA.md).

## Before You Start (deeper changes)

Please read:

- [README.md](README.md)
- [traverse-framework/.github](https://github.com/traverse-framework/.github) — constitution, quality standards, antipatterns, compatibility policy, exception process, CLA (this repo has adopted governance version 1.0.0)

## Core Rules

- Approved specs are versioned, immutable, and merge-gating for runtime/contract changes.
- Contracts are the source of truth for runtime behavior.
- Core runtime and business logic require 100% automated coverage.
- Material architecture changes require an ADR.
- Portability exceptions must be explicit and reviewed.
- All contributions are governed by the CLA at `traverse-framework/.github/CLA.md`.

## Workflow

1. Start from the governing approved spec (or confirm `no-spec-needed` on the issue).
2. Confirm whether an issue already exists.
3. Open or link the work item in the project board:
   [GitHub Project](https://github.com/orgs/traverse-framework/projects/1)
4. If needed, add or update an ADR before implementation.
5. Implement with tests and validation evidence.
6. Make sure the change passes the required validation flow.

## Pull Requests

Every pull request should:

- reference the governing spec version **or** state `no-spec-needed` with the issue link
- reference the relevant issue or work item
- explain any contract changes
- explain any compatibility impact
- explain any exception being used, if any

Pull requests should not merge if:

- implementation drifts from spec
- required tests or checks fail
- a required ADR is missing

## Issues

Use the issue templates when possible so work lands in the project board cleanly.
