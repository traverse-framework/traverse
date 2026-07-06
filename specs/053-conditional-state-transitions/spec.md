# Feature Specification: Conditional State Transitions

**Feature Branch**: `053-conditional-state-transitions`
**Created**: 2026-07-05
**Status**: Approved
**Input**: GitHub issue #536, extending the state-machine manifest slice approved in `052-app-state-machine`.

## Purpose

Traverse application manifests MAY route `capability_succeeded` transitions by evaluating deterministic predicates against the capability output. This lets a runtime-owned app state machine branch without duplicating transition logic in clients.

## Requirements

- **FR-001**: A transition MAY include a `condition` object.
- **FR-002**: `condition.field` MUST be a non-empty dot path rooted at `output`.
- **FR-003**: `condition.op` MUST be one of `eq`, `neq`, `gt`, `gte`, `lt`, `lte`, `in`, or `exists`.
- **FR-004**: Operators other than `exists` MUST include `condition.value`.
- **FR-005**: Transitions with the same `on` event MUST be evaluated in manifest order; the first matching condition wins.
- **FR-006**: A transition without `condition` is an unconditional fallback and MUST be evaluated after conditional transitions for the same `on` event.
- **FR-007**: If no condition matches a `capability_succeeded` event, runtime execution MUST emit a structured `no_matching_transition` error event and leave the session state unchanged.
- **FR-008**: Type mismatch during condition evaluation MUST emit a structured `condition_type_error` event and leave the session state unchanged.
- **FR-009**: `traverse-cli app validate` MUST reject malformed condition schemas before app registration.

## Schema Shape

```json
{
  "on": "capability_succeeded",
  "condition": {
    "field": "output.confidence_score",
    "op": "gte",
    "value": 0.85
  },
  "to": "auto_approved"
}
```

## Operator Semantics

| Operator | Meaning |
| --- | --- |
| `eq` | field value equals `value` |
| `neq` | field value does not equal `value` |
| `gt` | field value is greater than `value` |
| `gte` | field value is greater than or equal to `value` |
| `lt` | field value is less than `value` |
| `lte` | field value is less than or equal to `value` |
| `in` | field value is contained in array `value` |
| `exists` | field exists and is not null |

## Out of Scope

- HTTP command dispatch endpoints.
- HTTP state subscription endpoints.
- Durable execution of state-machine sessions.
- Capability output contract introspection warnings for condition paths.
