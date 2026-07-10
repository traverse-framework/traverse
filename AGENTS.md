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
# Add label
gh issue edit <NUMBER> --repo traverse-framework/traverse --add-label "agent:codex"

# Get project item ID with bounded output
gh project item-list 1 --owner traverse-framework --format json --limit 300 \
  --jq '.items[] | select(.content.number == <NUMBER>) | .id'

# Set Agent → Codex
gh project item-edit --project-id PVT_kwHOAEZXvs4BS6Ns \
  --id <ITEM_ID> \
  --field-id PVTSSF_lAHOAEZXvs4BS6NszhBK-Qk \
  --single-select-option-id 34d6db7d

# Set Status → In Progress
gh project item-edit --project-id PVT_kwHOAEZXvs4BS6Ns \
  --id <ITEM_ID> \
  --field-id PVTSSF_lAHOAEZXvs4BS6NszhATmdM \
  --single-select-option-id 47fc9ee4
```
