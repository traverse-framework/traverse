# Traverse — Agent Coordination

Canonical agent instructions (scope, structure, commands, style, governance) live in [CLAUDE.md](CLAUDE.md). This file holds only multi-tool coordination, per the org rule in `traverse-framework/.github` `docs/ai-agent-hardening.md`: **claim before you code — one issue = one agent.**

## Agent Coordination

**Before starting any work on an issue**, run these pre-flight checks:

### 1. Check for Claude Code claim

```bash
gh issue view <NUMBER> --repo traverse-framework/traverse --json labels
```

If the labels include `agent:claude` → **STOP**. Report:
> Issue #\<NUMBER\> is claimed by Claude Code. Choose a different ticket.

### 2. Check for Claude Code branch

```bash
git ls-remote --heads origin | grep "issue-<NUMBER>-"
```

If a `claude/issue-<NUMBER>-*` branch exists → **STOP**. Report:
> A Claude Code branch already exists for issue #\<NUMBER\>. Choose a different ticket.

### 3. Claim the ticket (only if pre-flight passes)

```bash
# Add label (the ownership marker — the live org Project 1 has no Agent field)
gh issue edit <NUMBER> --repo traverse-framework/traverse --add-label "agent:codex"

# Get project item ID with bounded output
# Keep Project reads in bounded pages: large `project item-list` GraphQL
# requests can exceed GitHub's per-query cost limit even with account quota.
gh project item-list 1 --owner traverse-framework --format json --limit 100 \
  --jq '.items[] | select(.content.number == <NUMBER>) | .id'

# Set Status → In Progress
gh project item-edit --project-id PVT_kwDOEbiBt84Bbyp1 \
  --id <ITEM_ID> \
  --field-id PVTSSF_lADOEbiBt84Bbyp1zhWglqM \
  --single-select-option-id 47fc9ee4
```

Status option IDs for reference: Ready `f75ad846`, In Progress `47fc9ee4`, Done `98236657`, Blocked `294b89f5`.

## Active Technologies
- Rust 1.95+; host packages in TypeScript, Swift, Kotlin, and .NET + Cargo workspace, serde, semver, existing registry and embedder packages (524-production-app-readiness)
- host-owned verified artifact cache, append-only trace journal, host-owned file-backed DataStore v2 (524-production-app-readiness)

## Recent Changes
- 524-production-app-readiness: Added Rust 1.95+; host packages in TypeScript, Swift, Kotlin, and .NET + Cargo workspace, serde, semver, existing registry and embedder packages
