# ADR-0069: Single-Role `service_type`; Compose Multi-Concern at Workflow

- Status: Accepted
- Date: 2026-09-11
- Governing specs: `014` / `208-service-type-taxonomy` (unchanged); `098-capability-event-host-abi`; `131-stateful-persistence-host-abi`
- Related issue: #1291
- Related: `docs/decision-log.md` Decision 68 (deferred multi-role); Decision 81

## Context

`service_type` is a single enum (`Stateless` | `Subscribable` | `Stateful`).
Spec `098` FR-003 gates `emit_event` to `Subscribable`. Spec `131` gates
`state_*` to `Stateful`. A capability therefore cannot both persist state and
participate in events. Session-flavored roster items (for example
`core.followup-session`, `identity.challenge-session`, `support.agent-session`)
plausibly want both concerns. Decision 68 deferred the multi-role taxonomy
question rather than inventing a combined type inside the Stateful ABI slice.
Spec `131` Compatibility already records multi-role as out of scope.

## Decision

Keep `service_type` as a single enum. Do **not** add a combined variant (for
example `StatefulSubscribable`) and do **not** introduce orthogonal contract
flags such as `uses_storage` / `participates_in_events`.

When a product need spans persistence and events, author **two** capabilities
(one `Stateful`, one `Subscribable`) and compose them at the workflow /
application layer. Call-time gates in `098` and `131` remain unchanged.
Placement rules in `014`/`208` continue to key off the single `service_type`
value.

If a concrete published capability later proves that workflow composition is
worse than a combined type, revisit with a **narrow** new enum variant — not
open-ended orthogonal flags.

## Consequences

- `#1291` is closed as a governing ruling, not an implementation ticket.
- No amendment to Approved Specs `014`/`208`, `098`, or `131` is required for
  this negative ruling; Spec `131` already excludes multi-role.
- Session-style products must split concerns across capabilities and workflows.
- Downstream Deferred Stateful ABI tickets (`state_clear`, quotas, CAS) remain
  independent of this taxonomy decision.

## Alternatives considered

- **New `service_type` variant** (e.g. `StatefulSubscribable`): rejected for
  now — every gate and placement rule would need to learn the new value, and
  no published multi-role workload is blocked today.
- **Orthogonal flags** beside or instead of the enum: rejected — Decision 68
  already noted that `208` placement is written against `service_type`; flags
  force a foundational rewrite without demand.
- **Address multi-role inside Spec `131`**: already rejected in Decision 68;
  this ADR closes the deferred follow-up with an explicit compose-at-workflow
  ruling.
