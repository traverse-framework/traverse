---
name: "speckit-constitution"
description: "Create or update the project constitution from interactive or provided principle inputs, ensuring all dependent templates stay in sync."
compatibility: "Requires spec-kit project structure with .specify/ directory"
metadata:
  author: "github-spec-kit"
  source: "templates/commands/constitution.md"
---


## User Input

```text
$ARGUMENTS
```

You **MUST** consider the user input before proceeding (if not empty).

## Outline

In this repo `.specify/memory/constitution.md` is **not** a full constitution and **not** a template. It is a pointer to the org constitution in `traverse-framework/.github` (`constitution.md`, at the release pinned in `.governance-version`) followed by a `## Traverse-Specific Rules` section. `scripts/ci/repository_checks.sh` fails if the pointer is removed, if either Traverse section is missing, or if an org principle heading (e.g. `### I. Capability-First Boundaries`) is copied back in.

Follow this execution flow:

1. Read `.specify/memory/constitution.md` and the org constitution:
   `gh api -H "Accept: application/vnd.github.raw" "repos/traverse-framework/.github/contents/constitution.md?ref=$(tr -d '[:space:]' < .governance-version)"`.

2. Decide where the requested change belongs:
   - **An org principle, Non-Functional Requirement, quality standard, workflow or amendment rule** (anything in the org constitution): do NOT edit this repo. Stop and tell the user the change must be proposed in `traverse-framework/.github` through its amendment process, then adopted here by bumping `.governance-version` (a maintainer step).
   - **A Traverse-only rule** (product scope, v0.1 limits, product constraints): continue.

3. Edit only the `## Traverse-Specific Rules` section. Never overwrite the file, never remove or reword the pointer paragraph, and never copy org principles, NFRs or quality standards into it. Traverse rules add to the org constitution and never relax it; if a requested rule would relax an org rule, stop and say so.

4. Record the change the way this repo amends governance: a spec amendment or a `docs/decision-log.md` entry that states the rule and why. Reference it in the PR body.

5. Check dependent artifacts for references to the changed rule: `.specify/templates/plan-template.md` (Constitution Check), `.specify/templates/spec-template.md`, `.specify/templates/tasks-template.md`, `README.md`, `AGENTS.md` and `CLAUDE.md`. Update only what the change affects.

6. Validate: `bash scripts/ci/repository_checks.sh` passes.

7. Output a short summary: which rule changed, where it is recorded, files updated, and a suggested commit message (e.g. `docs: amend Traverse-specific rule <name>`).
