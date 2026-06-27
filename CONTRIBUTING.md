# Contributing

Thanks for contributing to Traverse.

## Before You Start

Please read:

- [README.md](/Users/piovese/Documents/cogolo/README.md)
- [.specify/memory/constitution.md](/Users/piovese/Documents/cogolo/.specify/memory/constitution.md)
- [docs/quality-standards.md](/Users/piovese/Documents/cogolo/docs/quality-standards.md)
- [docs/antipatterns.md](/Users/piovese/Documents/cogolo/docs/antipatterns.md)
- [docs/compatibility-policy.md](/Users/piovese/Documents/cogolo/docs/compatibility-policy.md)
- [docs/exception-process.md](/Users/piovese/Documents/cogolo/docs/exception-process.md)

## Core Rules

- Approved specs are versioned, immutable, and merge-gating.
- Contracts are the source of truth for runtime behavior.
- Core runtime and business logic require 100% automated coverage.
- Material architecture changes require an ADR.
- Portability exceptions must be explicit and reviewed.

## Workflow

1. Start from the governing approved spec.
2. Confirm whether an issue already exists.
3. Open or link the work item in the project board:
   [GitHub Project](https://github.com/orgs/traverse-framework/projects)
4. If needed, add or update an ADR before implementation.
5. Implement with tests and validation evidence.
6. Make sure the change passes the required validation flow.

## Pull Requests

Every pull request should:

- reference the governing spec version
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
