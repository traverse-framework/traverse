# Spec Reviewer Guide

This template is filled in by the spec **author** and posted as a PR comment when requesting external review. Reviewers respond with the checklist at the bottom.

Review window: **72 hours** from the time this comment is posted. See [`docs/project-management.md`](project-management.md) for the full review policy.

---

## Reviewer Request Template (author fills this in)

```markdown
## Spec Review Request: <spec-id> — <spec-name>

**Review window closes**: YYYY-MM-DD HH:MM UTC (72h from now)
**Reviewers requested**: @reviewer1, @reviewer2, @reviewer3

---

### Summary

One paragraph: what problem does this spec solve and what does it govern?

---

### Problem Statement

- What is broken or missing today?
- Who is blocked by the absence of this spec?
- What is the failure mode without it?

---

### Success Criteria

What does "done" look like? List 3–5 observable outcomes that prove the spec works:

- [ ] ...
- [ ] ...
- [ ] ...

---

### Non-Goals

Explicit list of things this spec intentionally does NOT cover:

- Not: ...
- Not: ...

---

### Migration / Compatibility Impact

- Does this change existing contracts, APIs, or CLI behavior?
- What is the upgrade path for existing consumers?
- Is this a breaking change? If yes, what is the semver strategy?

---

### Threat Model Summary

- What can go wrong if this spec is wrong?
- What is the blast radius of a spec defect? (e.g., silent data loss, security bypass, irreversible state)
- What invariants must hold?

---

### Runnable Example

A command someone can actually run today (or after the PR merges) to see this working:

```bash
# Example:
cargo run -p traverse-cli-rs -- bundle inspect examples/expedition/registry-bundle/manifest.json
```

Expected output: <describe what success looks like>

---
```

---

## Reviewer Checklist (reviewer fills this in)

Copy this block into your review comment and answer each question:

```markdown
## Review: <spec-id> — <your GitHub handle>

**Verdict**: [ ] Approve  [ ] Approve with changes  [ ] Reject

---

| Question | Answer |
|---|---|
| Is the problem statement clear and accurate? | Yes / No / Unclear |
| Are the success criteria testable? | Yes / No / Unclear |
| Are the non-goals sufficient to prevent scope creep? | Yes / No |
| Is the compatibility impact acceptable? | Yes / No / N/A |
| Is the threat model honest? (Are risks understated?) | Yes / No |
| Does the runnable example work as described? | Yes / No / Not tested |

**Concerns** (if any):

> ...

**Specific change requests** (if Approve with changes):

> ...
```
