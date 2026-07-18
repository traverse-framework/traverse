# Event Publishing Tutorial

This tutorial shows how a capability author emits a governed internal event and how a subscriber capability receives it.

It follows the governed event model in Traverse: events are catalog-registered, lifecycle-enforced, and delivered synchronously through the `InProcessBroker`. No external broker or message queue is involved.

**Governing specs**

- [`specs/003-event-contracts/spec.md`](../specs/003-event-contracts/spec.md) — event contract artifact model
- [`specs/018-event-driven-composition/spec.md`](../specs/018-event-driven-composition/spec.md) — event-driven workflow composition

---

## Prerequisites

- Rust 1.94 or later
- the repository checked out locally
- a shell that can run the CI validation scripts

Confirm the workspace builds before starting:

```bash
cargo build
```

If you have not run the repository checks yet, do that first:

```bash
bash scripts/ci/repository_checks.sh
```

---

## What is an event in Traverse?

An event in Traverse is a governed, versioned signal emitted by a capability after it completes meaningful work. Events are not fire-and-forget notifications. Each event must:

- be defined in an immutable `contract.json` artifact
- be registered in the `EventCatalog` with an `Active` lifecycle before any capability can publish it
- be published through the `EventBroker` trait, which enforces catalog membership and lifecycle at publish time
- carry a structured JSON payload that matches the schema declared in its contract

Events advance workflow executions through explicit event-driven edges (spec 018). They do not create new executions and do not reach external systems in this governed slice.

The runtime types live in `crates/traverse-runtime/src/events/`:

| Type | Role |
|---|---|
| `TraverseEvent` | The event value: id, source, type, payload, and governance metadata |
| `EventCatalog` | Thread-safe registry of known event types |
| `EventCatalogEntry` | Metadata for one registered event type |
| `EventBroker` (trait) | Pub/sub interface — `publish`, `subscribe`, `unsubscribe` |
| `InProcessBroker` | Synchronous in-memory implementation of `EventBroker` |
| `EventError` | Errors returned by broker operations |
| `LifecycleStatus` | `Draft`, `Active`, or `Deprecated` |

---

## Step 1: Define the event contract

Every event must have a `contract.json` before any Rust code references it.

Create a contract file at the canonical path for your domain. Using the expedition domain as a reference:

```
contracts/examples/expedition/events/expedition-objective-captured/contract.json
```

The minimum required fields are:

```json
{
  "kind": "event_contract",
  "schema_version": "1.0.0",
  "id": "<namespace>.<name>",
  "namespace": "<namespace>",
  "name": "<name>",
  "version": "1.0.0",
  "lifecycle": "active",
  "owner": {
    "team": "<team-name>",
    "contact": "<contact-email>"
  },
  "summary": "<one-line human summary>",
  "description": "<longer description>",
  "payload": {
    "schema": {
      "type": "object",
      "required": ["<required-field>"],
      "properties": {
        "<required-field>": { "type": "string" }
      }
    },
    "compatibility": "backward-compatible"
  },
  "classification": {
    "domain": "<domain>",
    "bounded_context": "<context>",
    "event_type": "domain",
    "tags": ["<tag>"]
  },
  "publishers": [
    {
      "capability_id": "<namespace>.<capability-name>",
      "version": "1.0.0"
    }
  ],
  "subscribers": [],
  "policies": [],
  "tags": ["<tag>"],
  "provenance": {
    "source": "greenfield",
    "author": "<github-username>",
    "created_at": "<ISO-8601-timestamp>"
  },
  "evidence": []
}
```

The `id` must equal `<namespace>.<name>` exactly — the validator rejects mismatches.

The `lifecycle` must be `active` before any capability can publish this event. The broker refuses to publish `Draft` or `Deprecated` events.

The `publishers` array must list the capability that owns this event. The broker does not enforce this array at runtime, but registries and governance tools use it for impact analysis.

See the canonical example at `contracts/examples/expedition/events/expedition-objective-captured/contract.json` for a complete, validated contract.

---

## Step 2: Register the event in the catalog

Before any capability can publish the event, its type must be registered in an `EventCatalog`.

```rust
use std::sync::Arc;
use traverse_runtime::events::{
    EventCatalog, EventCatalogEntry, InProcessBroker, LifecycleStatus,
};

fn build_catalog() -> Result<Arc<EventCatalog>, traverse_runtime::events::EventError> {
    let catalog = Arc::new(EventCatalog::new());

    catalog.register(EventCatalogEntry {
        event_type: "expedition.planning.expedition-objective-captured".to_owned(),
        owner: "expedition.planning.capture-expedition-objective".to_owned(),
        version: "1.0.0".to_owned(),
        lifecycle_status: LifecycleStatus::Active,
        consumer_count: 0,
    })?;

    Ok(catalog)
}
```

Key rules enforced by `EventCatalog::register`:

- the `event_type` string must match the `id` field in `contract.json`
- registering the same `event_type` twice returns `EventError::LifecycleViolation`
- `lifecycle_status` must be `LifecycleStatus::Active` for publish operations to succeed

Use one catalog instance per runtime session and share it through `Arc<EventCatalog>`. The catalog is thread-safe.

---

## Step 3: Emit an event from a capability

Once the catalog is set up, create an `InProcessBroker` and publish from the emitting capability.

```rust
use serde_json::json;
use traverse_runtime::events::{
    EventBroker, InProcessBroker, LifecycleStatus, TraverseEvent,
};

fn emit_objective_captured(
    broker: &impl EventBroker,
    objective_id: &str,
) -> Result<(), traverse_runtime::events::EventError> {
    let event = TraverseEvent {
        id: uuid::Uuid::new_v4().to_string(),
        source: "traverse-runtime/expedition.planning.capture-expedition-objective".to_owned(),
        event_type: "expedition.planning.expedition-objective-captured".to_owned(),
        datacontenttype: "application/json".to_owned(),
        time: chrono::Utc::now().to_rfc3339(),
        data: json!({
            "objective_id": objective_id,
            "destination": "Mont Blanc",
            "target_window": {
                "start": "2026-07-01T00:00:00Z",
                "end": "2026-07-14T00:00:00Z"
            },
            "preferences": {
                "style": "alpine",
                "risk_tolerance": "moderate",
                "priority": "summit"
            },
            "notes": "acclimatization days required"
        }),
        owner: "expedition.planning.capture-expedition-objective".to_owned(),
        version: "1.0.0".to_owned(),
        lifecycle_status: LifecycleStatus::Active,
    };

    broker.publish(event)
}
```

What happens inside `InProcessBroker::publish`:

1. The broker looks up `event_type` in the catalog. If the type is not registered, it returns `EventError::UnregisteredEventType`.
2. If the catalog entry is `Draft` or `Deprecated`, it returns `EventError::LifecycleViolation`.
3. If the entry is `Active`, the broker calls every registered subscriber handler synchronously on the caller's thread.
4. Returns `Ok(())` when all handlers have been called.

The payload in `data` must conform to the `payload.schema` in the contract. The broker does not re-validate the schema at runtime, but downstream registry and CI tools do.

---

## Step 4: Register a subscriber capability

A subscriber registers a handler function before the publisher emits. Handlers are called synchronously in `publish` order.

```rust
use traverse_runtime::events::{EventBroker, TraverseEvent};

fn register_intent_interpreter(
    broker: &impl EventBroker,
) -> Result<(), traverse_runtime::events::EventError> {
    broker.subscribe(
        "expedition.planning.expedition-objective-captured",
        Box::new(|event: &TraverseEvent| {
            // The handler receives a shared reference to the event.
            // It must not panic. Use structured error handling or logging.
            let destination = event.data.get("destination")
                .and_then(|v| v.as_str())
                .unwrap_or("<unknown>");

            // In a real capability this would trigger downstream work,
            // update workflow state, or append to trace evidence.
            let _ = destination;
        }),
    )
}
```

Key rules for subscriber handlers:

- The handler signature is `Fn(&TraverseEvent) + Send + Sync`. The closure must be thread-safe.
- `broker.subscribe` returns `EventError::UnregisteredEventType` if the event type is not yet in the catalog. Register the event type in the catalog before subscribing.
- Handlers are called in registration order. Multiple subscribers for the same event type each receive the event exactly once per `publish` call.
- Handlers must not panic. The broker does not catch panics from handlers.

To remove all subscribers for an event type:

```rust
broker.unsubscribe("expedition.planning.expedition-objective-captured")?;
```

---

## Step 5: Run the workflow and observe event propagation

Putting the four pieces together in a minimal end-to-end path:

```rust
use std::sync::Arc;
use traverse_runtime::events::{
    EventBroker, EventCatalog, EventCatalogEntry, InProcessBroker, LifecycleStatus,
};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Build the catalog.
    let catalog = Arc::new(EventCatalog::new());
    catalog.register(EventCatalogEntry {
        event_type: "expedition.planning.expedition-objective-captured".to_owned(),
        owner: "expedition.planning.capture-expedition-objective".to_owned(),
        version: "1.0.0".to_owned(),
        lifecycle_status: LifecycleStatus::Active,
        consumer_count: 0,
    })?;

    // 2. Build the broker.
    let broker = InProcessBroker::new(catalog);

    // 3. Register the subscriber before publishing.
    register_intent_interpreter(&broker)?;

    // 4. Emit the event from the publisher capability.
    emit_objective_captured(&broker, "obj-001")?;

    Ok(())
}
```

The subscriber handler runs synchronously inside `emit_objective_captured` on the same thread. When `broker.publish` returns `Ok(())`, all registered handlers have already been called.

To validate the full governed expedition event path using the checked-in CLI:

```bash
# Register the expedition bundle (includes event contracts).
cargo run -p traverse-cli-rs -- bundle register \
  examples/expedition/registry-bundle/manifest.json

# Execute the canonical expedition request, which triggers the full
# workflow including event-driven progression.
cargo run -p traverse-cli-rs -- expedition execute \
  examples/expedition/runtime-requests/plan-expedition.json
```

To run the event-driven workflow smoke test:

```bash
bash scripts/ci/event_driven_workflow_smoke.sh
```

---

## Troubleshooting

### Subscriber handler is never called

- Confirm `broker.subscribe` was called before `broker.publish`. Subscriptions registered after `publish` do not receive the already-emitted event.
- Check that the `event_type` string in `broker.subscribe` exactly matches the `event_type` in the `TraverseEvent` published. The match is case-sensitive and exact.

### `EventError::UnregisteredEventType` on publish

- The `event_type` in `TraverseEvent.event_type` is not registered in the `EventCatalog`.
- Call `catalog.register(...)` before calling `broker.publish` or `broker.subscribe`.
- Confirm the string matches the `id` field in the event `contract.json`.

### `EventError::LifecycleViolation` on publish

- The catalog entry for the event type has `lifecycle_status: Draft` or `lifecycle_status: Deprecated`.
- Set `lifecycle_status: LifecycleStatus::Active` in the `EventCatalogEntry` when registering the event type.
- If the event is genuinely deprecated, retire it from the publishing path and update the contract `lifecycle` field to `deprecated`.

### `EventError::LifecycleViolation` — "already registered"

- `catalog.register` was called twice with the same `event_type`. The catalog enforces uniqueness.
- Register each event type exactly once per catalog instance. If you need to reload the catalog, create a new `EventCatalog`.

### Subscriber receives the event but panics

- Broker handlers must not panic. A panic inside a handler is not caught and will propagate to the caller of `broker.publish`.
- Replace any `unwrap()` or `expect()` calls inside the handler with pattern matching or `if let`.

### CI gate fails after adding a new event contract

- Run `bash scripts/ci/spec_alignment_check.sh` to see which spec alignment rule the new contract violates.
- Confirm the contract `kind`, `schema_version`, `lifecycle`, `id`, `namespace`, and `name` fields satisfy the rules in spec 003.
- The alignment gate checks that every event contract in `contracts/` references a registered capability publisher.

---

## Related docs

- [`docs/getting-started.md`](getting-started.md) — first capability path for new developers
- [`docs/expedition-example-authoring.md`](expedition-example-authoring.md) — full governed expedition artifact set
- [`docs/wasm-agent-authoring-guide.md`](wasm-agent-authoring-guide.md) — how to create WASM agents that emit events
- [`docs/multi-thread-workflow.md`](multi-thread-workflow.md) — parallel agent workflow coordination
- [`specs/003-event-contracts/spec.md`](../specs/003-event-contracts/spec.md) — governing spec for event contract artifacts
- [`specs/018-event-driven-composition/spec.md`](../specs/018-event-driven-composition/spec.md) — governing spec for event-driven workflow progression

---

## Connecting Contract Declaration to Runtime Emission (#292)

The `emits` and `consumes` fields in a capability contract are the governance bridge between what a contract declares and what the runtime enforces at execution time.

### The same event from three perspectives

**1. Contract declaration** — in `contracts/your-domain/capabilities/say-hello/contract.json`:
```json
{
  "emits": ["examples.hello-world.greeted"],
  "consumes": []
}
```
This declares that this capability _may_ publish the `examples.hello-world.greeted` event. It is a promise to the registry.

**2. Runtime emission** — inside the WASM binary or native executor:
```rust
let event = TraverseEvent {
    event_type: "examples.hello-world.greeted".to_string(),
    payload: serde_json::json!({ "subject": "Alice", "greeting": "Hello, Alice!" }),
    // ...
};
broker.publish(event)?;
```
At runtime, `broker.publish()` checks the active event catalog. If `examples.hello-world.greeted` is not registered in the catalog, it returns `EventError::UnknownEventType`. If it _is_ registered but the capability contract does not declare it in `emits`, it returns `EventError::PolicyViolation`.

**3. Subscriber registration** — a downstream capability or handler:
```rust
broker.subscribe("examples.hello-world.greeted", Box::new(|event| {
    // handle the event
}));
```
Subscribers are registered before execution begins. The runtime delivers published events synchronously to all registered subscribers in registration order.

### What the runtime validates at each stage

| Stage | Validation |
|-------|------------|
| Bundle registration | `emits` event IDs must exist in the event catalog |
| Contract parse | `emits` and `consumes` must be valid event contract ID strings |
| `broker.publish()` | Event type must be in catalog AND declared in capability's `emits` |
| Subscription | No validation — subscribers are registered independently |

### Common mistake: publishing without declaring

If your capability calls `broker.publish("my.event")` but the contract does not list `"my.event"` in `emits`, the runtime returns `EventError::PolicyViolation`. Always keep `emits` in the contract in sync with `broker.publish()` calls in the implementation.
