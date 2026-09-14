# Contributing

Thanks for contributing to Traverse.

## First contributions (start here)

1. Pick an open org issue labeled [`good first issue`](https://github.com/issues?q=is%3Aopen+is%3Aissue+org%3Atraverse-framework+label%3A%22good+first+issue%22) or [`help wanted`](https://github.com/issues?q=is%3Aopen+is%3Aissue+org%3Atraverse-framework+label%3A%22help+wanted%22).
2. Comment on the issue so we know you are taking it.
3. Open a focused PR that links the issue. In the PR body, fill in **## Governing Spec** and **## Validation** — even for `no-spec-needed` tickets. If you touch a governed path, declare that path's approved spec; otherwise say `no-spec-needed` and follow the issue / repo convention (no new `specs/` slice required).

By contributing, you accept the [CLA](https://github.com/traverse-framework/.github/blob/main/CLA.md). Please also read [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) and [SECURITY.md](SECURITY.md).

Deeper rules for runtime, contracts, coverage, and ADRs are below — and in [org governance](https://github.com/traverse-framework/.github). Do not treat the constitution as the onboarding doc.

Good first shapes right now: MCP client config docs, TypeScript/Python consume examples, website polish, one small registry capability, skill-template wording.

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

- include **## Governing Spec** and **## Validation** in the body (template sections) — declare the governing approved spec for any touched governed path, or state `no-spec-needed` with the issue link
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
