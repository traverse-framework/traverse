# Event Contract Authoring Guide

This guide shows how to author a Traverse **event contract** from scratch.

Use the checked-in examples as living references:

- [`contracts/examples/expedition/events/`](../contracts/examples/expedition/events/)
- [`specs/003-event-contracts/data-model.md`](../specs/003-event-contracts/data-model.md)

## What An Event Contract Is

An event contract is the governed source of truth for an event type:

- identity (`id`, `version`)
- payload schema + compatibility signal (`payload.schema`, `payload.compatibility`)
- classification metadata (`classification`)
- publisher and subscriber edges (`publishers`, `subscribers`)

Traverse uses event contracts for validation, registry integrity, and workflow/event-driven composition.

## Minimal Working Template

This is a minimal event contract you can copy, edit, and validate locally.

```json
{
  "kind": "event_contract",
  "schema_version": "1.0.0",
  "id": "demo.echoed",
  "namespace": "demo",
  "name": "echoed",
  "version": "1.0.0",
  "lifecycle": "draft",
  "owner": { "team": "your-team", "contact": "you@example.com" },
  "summary": "A demo echo response was produced.",
  "description": "Minimal event contract used to validate authoring and registration wiring.",
  "payload": {
    "schema": {
      "type": "object",
      "required": ["message"],
      "properties": { "message": { "type": "string" } },
      "additionalProperties": false
    },
    "compatibility": "backward-compatible"
  },
  "classification": {
    "domain": "demo",
    "bounded_context": "core",
    "event_type": "domain",
    "tags": ["demo"]
  },
  "publishers": [
    { "capability_id": "demo.echo", "version": "1.0.0" }
  ],
  "subscribers": [],
  "policies": [{ "id": "manual-approval-required" }],
  "tags": ["demo", "example"],
  "provenance": {
    "source": "greenfield",
    "author": "your-handle",
    "created_at": "2026-04-18T00:00:00Z"
This guide shows how to author a valid event contract for Traverse, connect it to a capability contract, and validate it locally.
- [`contracts/examples/expedition/events/expedition-objective-captured/contract.json`](../contracts/examples/expedition/events/expedition-objective-captured/contract.json)
- [`specs/003-event-contracts/spec.md`](../specs/003-event-contracts/spec.md)
- [`docs/event-publishing-tutorial.md`](event-publishing-tutorial.md)
---
## Minimal Annotated Template
Place your event contract at a path that follows the convention:
```
contracts/<domain>/events/<event-name>/contract.json
```
  "kind": "event_contract",              // must be exactly "event_contract"
  "schema_version": "1.0.0",            // must be "1.0.0" in v0.1
  "id": "acme.orders.order-placed",     // must equal namespace.name exactly
  "namespace": "acme.orders",           // dot-separated lowercase kebab-case
  "name": "order-placed",               // lowercase kebab-case
  "version": "1.0.0",                   // semver MAJOR.MINOR.PATCH
  "lifecycle": "active",                // use "draft" until validation passes
  "owner": {
    "team": "orders-team",
    "contact": "orders@example.com"
  "summary": "An order has been placed and is ready for fulfillment.",
  "description": "Emitted by the place-order capability after all order fields pass validation and the order is durably recorded.",
    "schema": {                          // JSON Schema describing the event payload
      "required": ["order_id", "placed_at"],
      "properties": {
        "order_id":  { "type": "string" },
        "placed_at": { "type": "string", "format": "date-time" }
      }
    "compatibility": "backward-compatible"  // "backward-compatible" or "breaking"
    "domain": "acme",
    "bounded_context": "orders",
    "event_type": "domain",             // "domain", "integration", or "command"
    "tags": ["orders", "placement"]
    {
      "capability_id": "acme.orders.place-order",
      "version": "1.0.0"
    }
  "subscribers": [],                    // optional; list known consumers
  "policies": [],
  "tags": ["orders", "example"],
    "source": "greenfield",             // "greenfield", "brownfield-extracted", "ai-generated", "ai-assisted"
    "author": "your-github-username",
    "created_at": "2026-04-17T00:00:00Z"
  },
  "evidence": []
}
```

## Field Notes

- `kind`: Must be `event_contract`.
- `schema_version`: Must be `1.0.0`.
- `id`: Globally unique string for the event type, typically `namespace.name`.
- `namespace` / `name`: Stable identity components.
- `version`: SemVer string. Increment when the event contract changes.
- `lifecycle`: Use `draft` while iterating; publishable flows require `active`.
- `payload.schema`: JSON Schema describing the event payload.
- `payload.compatibility`: Declares payload change compatibility; use `backward-compatible` unless you have a reason not to.
- `classification`: Metadata used for discovery and documentation.
- `publishers`: Capabilities allowed to emit this event (identity + version).
- `subscribers`: Capabilities that subscribe to this event (identity + version).
- `policies`: Governance policy identifiers (for example, manual approval).
- `tags`: Search / organization tags.
- `provenance`: Traceability metadata.
- `evidence`: Validation evidence records (often empty for a new draft).

## Lifecycle Values

| Value        | Meaning                                         |
|--------------|-------------------------------------------------|
| `draft`      | Not publishable for registry/runtime use        |
| `active`     | Eligible for registry and runtime use           |
| `deprecated` | Still valid but discouraged for new composition |
| `retired`    | No longer eligible for new selection            |
| `archived`   | Retained as historical record only              |

## Authoring Steps (Create → Validate → Register)

1. Choose `namespace`, `name`, and compute `id = namespace.name`.
2. Start with `lifecycle: draft`.
3. Define a strict `payload.schema`.
4. Add at least one `publisher` capability reference once the emitting capability exists.
5. Validate locally:

```bash
cargo test -p traverse-contracts
```

6. Inspect an event contract via the CLI (this should fail fast if malformed):

```bash
cargo run -p traverse-cli-rs -- event inspect <path-to-contract.json>
```

7. Add the event contract to a bundle manifest and register it:

```bash
cargo run -p traverse-cli-rs -- bundle register <path-to-manifest.json>
```

## Common Mistakes

- Using permissive payload schemas that undermine determinism.
- Forgetting to update `publishers` when the emitting capability version changes.
- Treating `tags` as a stability boundary. They are discoverability metadata, not identity.

---
## Required Fields
| Field | Type | Rule |
|---|---|---|
| `kind` | string | must be `event_contract` |
| `schema_version` | string | must be `1.0.0` |
| `id` | string | must equal `namespace.name` exactly |
| `namespace` | string | dot-separated lowercase kebab-case |
| `name` | string | lowercase kebab-case |
| `version` | string | semver `MAJOR.MINOR.PATCH` |
| `lifecycle` | string | one of `draft`, `active`, `deprecated`, `retired`, `archived` |
| `owner.team` | string | stable ownership identifier |
| `owner.contact` | string | non-empty contact |
| `summary` | string | 10–200 characters; one meaningful business event description |
| `description` | string | at least 20 characters |
| `payload.schema` | object | JSON Schema-compatible object describing the payload shape |
| `payload.compatibility` | string | `backward-compatible` or `breaking` |
| `classification.domain` | string | domain name |
| `classification.bounded_context` | string | bounded context within the domain |
| `classification.event_type` | string | `domain`, `integration`, or `command` |
| `publishers` | array | at least one capability declared as publisher |
| `publishers[].capability_id` | string | must reference a registered capability |
| `publishers[].version` | string | semver |
| `provenance.source` | string | one of `greenfield`, `brownfield-extracted`, `ai-generated`, `ai-assisted` |
| `provenance.author` | string | GitHub username or team handle |
| `provenance.created_at` | string | ISO 8601 timestamp |
| `subscribers` | array | required; may be empty |
| `policies` | array | required; may be empty |
| `tags` | array | required; may be empty |
| `evidence` | array | required; may be empty |
---
## Connecting to a Capability Contract
Events and capabilities are connected through the `emits` and `consumes` arrays in the capability contract and the `publishers` and `subscribers` arrays in the event contract.
### Emitting capability
The capability that fires the event declares it in its `emits` array:
```json
"emits": [
  {
    "event_id": "acme.orders.order-placed",
    "version": "1.0.0"
  }
]
The event contract's `publishers` array must list that same capability:
```json
"publishers": [
  {
    "capability_id": "acme.orders.place-order",
    "version": "1.0.0"
  }
]
The spec-alignment gate checks that every event contract in `contracts/` references at least one registered publisher.
### Consuming capability
A capability that subscribes to the event declares it in its `consumes` array:
```json
"consumes": [
  {
    "event_id": "acme.orders.order-placed",
    "version": "1.0.0"
  }
]
Optionally, list it in the event contract's `subscribers` array for documentation and impact analysis:
```json
"subscribers": [
  {
    "capability_id": "acme.fulfillment.start-fulfillment",
    "version": "1.0.0"
  }
]
---
## How to Validate Locally
Run the CLI inspect command against your event contract:
cargo run -p traverse-cli-rs -- event inspect contracts/path/to/event-name/contract.json
Expected output includes `id`, `version`, `lifecycle`, publisher count, and subscriber count. Any structural error is printed to stderr with a non-zero exit code.
Then run the full spec-alignment gate:
bash scripts/ci/spec_alignment_check.sh
bash scripts/ci/repository_checks.sh
---
## The Same Event from Three Perspectives
The following example traces the `expedition.planning.expedition-objective-captured` event through its full lifecycle: contract declaration, runtime emission, and subscriber registration. This corresponds to the pattern described in issue #292.
### 1. Contract declaration
`contracts/examples/expedition/events/expedition-objective-captured/contract.json` declares:
- `id`: `expedition.planning.expedition-objective-captured`
- `publishers`: `expedition.planning.capture-expedition-objective@1.0.0`
- `payload.schema`: the shape of the structured objective record
The capability contract at `contracts/examples/expedition/capabilities/capture-expedition-objective/contract.json` mirrors this with:
```json
"emits": [
  {
    "event_id": "expedition.planning.expedition-objective-captured",
    "version": "1.0.0"
  }
]
The `emits` declaration in the capability contract is authoritative. The runtime broker constrains what `broker.publish()` may emit: **if an event type is not registered in the catalog as `Active`, `broker.publish` returns `EventError::LifecycleViolation` and the event is not delivered.**
### 2. Runtime emission
The emitting capability registers the event type in an `EventCatalog` with `LifecycleStatus::Active`, then publishes through the `InProcessBroker`:
```rust
catalog.register(EventCatalogEntry {
    event_type: "expedition.planning.expedition-objective-captured".to_owned(),
    owner: "expedition.planning.capture-expedition-objective".to_owned(),
    version: "1.0.0".to_owned(),
    lifecycle_status: LifecycleStatus::Active,
    consumer_count: 0,
})?;
let broker = InProcessBroker::new(catalog);
broker.publish(TraverseEvent {
    event_type: "expedition.planning.expedition-objective-captured".to_owned(),
    // ... other fields matching the contract payload schema
})?;
### 3. Subscriber registration
A downstream capability subscribes before the event is emitted:
```rust
broker.subscribe(
    "expedition.planning.expedition-objective-captured",
    Box::new(|event: &TraverseEvent| {
        // handle the event payload
    }),
)?;
`broker.subscribe` returns `EventError::UnregisteredEventType` if the event type is not in the catalog. Subscriptions must be registered before `broker.publish` is called — handlers registered after the event is emitted do not receive it.
---
- Setting `id` to anything other than `namespace.name` — the validator rejects mismatches.
- Declaring `lifecycle: active` before the event is fully specified — use `draft` during authoring.
- Omitting the capability from `publishers` — the spec-alignment gate flags contracts with no declared publisher.
- Using a `payload.schema` that does not match the data actually emitted by the capability — the broker does not re-validate the schema at runtime, but CI tools do.
- Subscribing to an event type that is not yet registered in the catalog — `broker.subscribe` returns an error.
---
## Related Documents
- [`docs/event-publishing-tutorial.md`](event-publishing-tutorial.md) — end-to-end tutorial for emitting and subscribing to events
- [`specs/003-event-contracts/spec.md`](../specs/003-event-contracts/spec.md) — governing spec
- [`specs/018-event-driven-composition/spec.md`](../specs/018-event-driven-composition/spec.md) — event-driven workflow edges
- [`docs/capability-contract-authoring-guide.md`](capability-contract-authoring-guide.md) — how `emits` connects to event contracts
