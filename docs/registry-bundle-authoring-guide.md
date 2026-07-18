# Registry Bundle Authoring Guide

A registry bundle is the unit by which capabilities, events, and workflows are registered into Traverse. Every artifact that the runtime resolves must be present in a registered bundle.

Use the canonical example as a living reference:

- [`examples/expedition/registry-bundle/manifest.json`](../examples/expedition/registry-bundle/manifest.json)
- [`docs/workflow-composition-guide.md`](workflow-composition-guide.md) — step-by-step bundle creation

---

## Full Annotated `manifest.json` Template

```json
{
  "bundle_id": "acme.orders.example-bundle",   // unique bundle identity; dot-separated
  "version": "1.0.0",                          // semver MAJOR.MINOR.PATCH
  "scope": "public",                           // "public" or "private"

  "capabilities": [
    {
      "id": "acme.orders.place-order",         // must match the "id" field in contract.json
      "version": "1.0.0",                      // must match the "version" field in contract.json
      "path": "../../../contracts/acme/orders/capabilities/place-order/contract.json"
                                               // relative path from this manifest file
    }
  ],

  "events": [
    {
      "id": "acme.orders.order-placed",        // must match the "id" field in the event contract
      "version": "1.0.0",
      "path": "../../../contracts/acme/orders/events/order-placed/contract.json"
    }
  ],

  "workflows": [
    {
      "id": "acme.orders.draft-and-confirm",   // must match the "id" field in workflow.json
      "version": "1.0.0",
      "path": "../../../workflows/acme/orders/draft-and-confirm/workflow.json"
    }
  ]
}
```

---

## Required vs Optional Fields

| Field | Required | Notes |
|---|---|---|
| `bundle_id` | Yes | Unique identity for this bundle. Dot-separated. |
| `version` | Yes | Semver. Bumping the version creates a new immutable bundle entry. |
| `scope` | Yes | `public` or `private`. Public bundles are visible to all consumers; private bundles are registry-scoped. |
| `capabilities` | Yes | May be an empty array `[]`, but the key must be present. |
| `capabilities[].id` | Yes | Must match the `id` field in the referenced `contract.json`. |
| `capabilities[].version` | Yes | Must match the `version` field in the referenced `contract.json`. |
| `capabilities[].path` | Yes | Relative path from the manifest file to the contract. |
| `events` | No | Omit the key entirely if no events are registered. An empty array is also acceptable. |
| `events[].id` | Yes (if present) | Must match the `id` field in the event contract. |
| `events[].version` | Yes (if present) | Must match the `version` field in the event contract. |
| `events[].path` | Yes (if present) | Relative path from the manifest file to the event contract. |
| `workflows` | No | Omit the key entirely if no workflows are registered. |
| `workflows[].id` | Yes (if present) | Must match the `id` field in the workflow definition. |
| `workflows[].version` | Yes (if present) | Must match the `version` field in the workflow definition. |
| `workflows[].path` | Yes (if present) | Relative path from the manifest file to the workflow definition. |

---

## How Capabilities, Events, and Workflows Are Referenced

The bundle manifest is a pointer document. It does not embed contract content — it references the canonical artifact files by relative path.

When the CLI loads a bundle:

1. It reads each contract at the given path.
2. It validates the `id` and `version` declared in the manifest entry against the `id` and `version` inside the artifact file. A mismatch causes a hard validation error.
3. It registers each artifact into the in-memory capability registry, event registry, and workflow registry respectively.
4. It validates inter-artifact references: a workflow that references a capability version checks that the capability is present in the loaded set; an event edge in a workflow checks that the event contract is registered.

Paths are resolved relative to the directory containing `manifest.json`, not relative to the repository root. The canonical expedition bundle uses `../../../contracts/...` because the manifest lives three directory levels below the root.

---

## How to Add a New Capability

1. Create the capability contract at the canonical path:

   ```
   contracts/<domain>/capabilities/<name>/contract.json
   ```

2. Add an entry to the bundle manifest:

   ```json
   {
     "id": "<namespace>.<name>",
     "version": "1.0.0",
     "path": "../../../contracts/<domain>/capabilities/<name>/contract.json"
   }
   ```

3. If the capability emits events, add the corresponding event contract entries to `events`.

4. If the capability participates in a workflow, ensure the workflow definition lists it as a node.

5. Validate the bundle before registering (see below).

---

## Bundle Validation Command

**Inspect** (validates and prints a summary without modifying registries):

```bash
cargo run -p traverse-cli-rs -- bundle inspect \
  examples/your-bundle/registry-bundle/manifest.json
```

Expected output includes `bundle_id`, `version`, `scope`, the count of discovered artifacts, and the individual capability/event/workflow ids.

**Register** (loads into in-memory registries for the current session):

```bash
cargo run -p traverse-cli-rs -- bundle register \
  examples/your-bundle/registry-bundle/manifest.json
```

Expected output includes `bundle_id`, `version`, `scope`, registered counts per type, and a summary record per registered artifact.

**Spec-alignment gate** (must pass before opening a PR):

```bash
bash scripts/ci/spec_alignment_check.sh
bash scripts/ci/repository_checks.sh
```

---

## Common Mistakes

- **Path not relative to manifest** — a path that is correct relative to the repository root but not relative to the manifest directory will fail with a file-not-found error at load time. Always compute paths relative to the manifest file's parent directory.
- **`id`/`version` mismatch between manifest and artifact** — the loader validates that the declared `id` and `version` in the manifest entry match the `id` and `version` inside the artifact file. Copying an entry without updating one of these fields is the most common cause of this error.
- **Missing event entries for capabilities that emit** — if a capability declares `emits` but the event contract is not in the bundle, the spec-alignment gate will flag the missing publisher reference.
- **Workflow references a capability not in the bundle** — the workflow registration validator checks that every `capability_id` referenced in a node is present in the registered capability set. Add all required capability entries to the bundle before registering the workflow.
- **Re-registering the same `(bundle_id, version)` with different content** — once a bundle version is registered with a given digest, it cannot be changed. Bump the `version` field to register updated content.

---

## Related Documents

- [`docs/workflow-composition-guide.md`](workflow-composition-guide.md) — end-to-end bundle creation walkthrough
- [`docs/capability-contract-authoring-guide.md`](capability-contract-authoring-guide.md) — how to write the capability contracts referenced in the bundle
- [`docs/event-contract-authoring-guide.md`](event-contract-authoring-guide.md) — how to write the event contracts referenced in the bundle
- [`docs/workflow-contract-authoring-guide.md`](workflow-contract-authoring-guide.md) — how to write the workflow definitions referenced in the bundle
- [`docs/cli-reference.md`](cli-reference.md) — full CLI command reference

---

## Registration Idempotency

`CapabilityRegistry::register()` is **idempotent** when the same version and digest are submitted more than once.

### Same version, same digest → succeeds silently

If a capability is re-registered with the exact same contract digest and artifact metadata as the version already in the registry, the call returns the existing `RegistrationOutcome` unchanged. The `already_registered` field on the outcome is set to `true` so callers can distinguish this path from a fresh registration.

No state is mutated. The operation is safe to retry any number of times.

### Same version, different digest → `ImmutableVersionConflict`

If the same version is submitted with a different contract digest or artifact metadata, the registry rejects it with `ImmutableVersionConflict`. Published versions are immutable. To ship a correction, publish a new semver version.

### Agent retry pattern

A common failure scenario: an agent registers a capability, then crashes before persisting the outcome. On restart it does not know whether registration succeeded.

Recommended pattern:

1. Attempt registration unconditionally.
2. If the result is `Ok(outcome)` and `outcome.already_registered == true`, log "already registered — continuing" and proceed.
3. If the result is `Ok(outcome)` and `outcome.already_registered == false`, this is a fresh registration — persist the outcome and proceed.
4. If the result is `Err(ImmutableVersionConflict)`, the same version was registered with a different digest. This is a real conflict and requires human review.

```rust
match registry.register(request) {
    Ok(outcome) if outcome.already_registered => {
        // safe to continue — idempotent retry
    }
    Ok(outcome) => {
        // first registration — persist outcome
    }
    Err(failure) => {
        // surface the error
    }
}
```

This pattern means agents never need a separate "check-then-register" round-trip, which would introduce a TOCTOU window.
