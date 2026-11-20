# Traverse — Agent Coordination

Canonical agent instructions (scope, structure, commands, style, governance) live in [CLAUDE.md](CLAUDE.md). This file holds only multi-tool coordination, per the org rule in `traverse-framework/.github` `docs/ai-agent-hardening.md`: **claim before you code — one issue = one agent.**

## Agent Coordination

Claim and release with the shared scripts, vendored from
`traverse-framework/.github` at the version in `.governance-version`. Don't run
the steps by hand. The ids live in
`.agent-claim.json`.

```bash
scripts/agent/claim.sh <issue> <claude|codex|cursor> --dry-run   # check only
scripts/agent/claim.sh <issue> <claude|codex|cursor>             # claim
scripts/agent/release.sh <issue> <Ready|Blocked|Done>            # release
```

`claim.sh` changes nothing unless the issue is open, its Project 1 status is
Ready, it carries no `agent:*` label, and no `*/issue-<N>-*` branch or open PR
from any tool exists. If it refuses, pick another ticket; never "help" a
claimed one. Work on the branch it prints (`<tool>/issue-<N>-...`).

## Active Technologies
- Rust 1.95+; host packages in TypeScript, Swift, Kotlin, and .NET + Cargo workspace, serde, semver, existing registry and embedder packages (524-production-app-readiness)
- host-owned verified artifact cache, append-only trace journal, host-owned file-backed DataStore v2 (524-production-app-readiness)

## Recent Changes
- 524-production-app-readiness: Added Rust 1.95+; host packages in TypeScript, Swift, Kotlin, and .NET + Cargo workspace, serde, semver, existing registry and embedder packages
