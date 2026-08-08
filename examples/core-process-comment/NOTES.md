# core.process-comment

Governed example for `core.process-comment@1.0.1` — policy-driven comment action processor.

## Run

```bash
bash scripts/ci/core_process_comment_example_smoke.sh
```

## Coverage

| ID | Fixture | Expected `reason_code` |
|----|---------|------------------------|
| UC-01 | `uc01-create-mentions-allow.json` | `ok` (+ mention notify) |
| UC-02 | `uc02-edit-not-owner-deny.json` | `not_owner` |
| UC-03 | `uc03-reply-depth-deny.json` | `max_thread_depth_exceeded` |
| UC-04 | `uc04-moderation-quarantine-allow.json` | `moderation_quarantine` |
| UC-05 | `uc05-react-allow.json` | `ok` (reaction) |
| UC-06 | `uc06-soft-delete-allow.json` | `ok` (soft-delete) |
| UC-07 | `uc07-tenant-isolation-deny.json` | `tenant_isolation_violation` |
| UC-08 | `uc08-empty-body-deny.json` | `empty_body` |

## Honesty

`1.0.1` narrows `action.enum` and description to the tested matrix. See ADR-0038 / Spec 102.
