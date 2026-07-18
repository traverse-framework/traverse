# Capability Contract Authoring Guide

This guide covers how to author a valid capability contract for Traverse, including a copy-pasteable minimal template and a complete reference for `execution.constraints`.

Use the checked-in examples as living references:

- [`contracts/examples/expedition/capabilities/`](../contracts/examples/expedition/capabilities/)
- [`contracts/examples/hello-world/capabilities/say-hello/contract.json`](../contracts/examples/hello-world/capabilities/say-hello/contract.json)
- [`specs/002-capability-contracts/data-model.md`](../specs/002-capability-contracts/data-model.md)

## Contract Structure

A capability contract is a `contract.json` artifact placed under `contracts/`. The top-level shape must include all required fields defined in spec `002-capability-contracts`. The key governed sections are:

- `kind` — must be `capability_contract`
- `schema_version` — must be `1.0.0` for v0.1
- `id`, `namespace`, `name` — identity triple; `id` must equal `namespace.name`
- `version` — semantic version `MAJOR.MINOR.PATCH`
- `lifecycle` — see lifecycle enum below
- `inputs` / `outputs` — JSON Schemas used for deterministic validation
- `execution` — binary format, entrypoint, preferred targets, and constraints

## Minimal Working Template

This is a minimal contract you can copy, edit, and validate locally. It intentionally avoids events and dependencies so you can focus on structure first.

```json
{
  "kind": "capability_contract",
  "schema_version": "1.0.0",
  "id": "demo.echo",
  "namespace": "demo",
  "name": "echo",
  "version": "1.0.0",
  "lifecycle": "draft",
  "owner": { "team": "your-team", "contact": "you@example.com" },
  "summary": "Echo the request payload.",
  "description": "Minimal contract used to validate authoring and registration wiring.",
  "inputs": {
    "schema": {
      "type": "object",
      "required": ["message"],
      "properties": { "message": { "type": "string" } },
      "additionalProperties": false
    }
  },
  "outputs": {
    "schema": {
      "type": "object",
      "required": ["message"],
      "properties": { "message": { "type": "string" } },
      "additionalProperties": false
    }
  },
  "preconditions": [{ "id": "input-provided", "description": "A message is provided." }],
  "postconditions": [{ "id": "echo-produced", "description": "The output contains the same message." }],
  "side_effects": [{ "kind": "none", "description": "No side effects." }],
  "emits": [],
  "consumes": [],
  "permissions": [{ "id": "demo.echo.execute" }],
  "execution": {
    "binary_format": "wasm",
    "entrypoint": { "kind": "wasi-command", "command": "run" },
    "preferred_targets": ["local"],
    "constraints": {
      "host_api_access": "none",
      "network_access": "forbidden",
      "filesystem_access": "none"
    }
  },
  "policies": [{ "id": "manual-approval-required" }],
  "dependencies": [],
  "provenance": {
    "source": "greenfield",
    "author": "your-handle",
    "created_at": "2026-04-18T00:00:00Z",
    "spec_ref": "002-capability-contracts@1.0.0",
    "adr_refs": [],
    "exception_refs": []
  },
  "evidence": [],
  "service_type": "stateless",
  "permitted_targets": ["local", "cloud", "edge", "device"]
}
```

Notes:

- `service_type` defaults to `stateless` if omitted, but setting it explicitly makes author intent clearer.
- If you set `host_api_access` to `exception_required`, validation requires at least one entry in `provenance.exception_refs`.

## Lifecycle Values

| Value        | Meaning                                         |
|--------------|-------------------------------------------------|
| `draft`      | Not publishable for runtime use                 |
| `active`     | Eligible for runtime use                        |
| `deprecated` | Still valid but discouraged for new composition |
| `retired`    | No longer eligible for new runtime selection    |
| `archived`   | Retained as historical record only              |

Only `active` and `deprecated` are runtime-eligible.

## Constraint Reference

Every capability contract's `execution` block must include a `constraints` object with exactly three fields. These fields describe the security and portability posture of the capability at runtime.

```json
"constraints": {
  "host_api_access": "none",
  "network_access": "forbidden",
  "filesystem_access": "none"
}
```

The tables below document all valid values, their meaning, and whether the runtime enforces the constraint or treats it as a declaration.

### `host_api_access`

Controls whether the WASM module may call host-provided APIs beyond standard WASI.

| Value                | Description                                                                                       | Runtime enforcement |
|----------------------|---------------------------------------------------------------------------------------------------|---------------------|
| `none`               | No host-specific API access. Fully portable across all execution targets.                         | Documentation-only in v0.1. The runtime does not inspect WASM imports at execution time. |
| `exception_required` | Host API access is required and must be justified by an approved portability exception reference. | Structurally enforced: validation rejects this without at least one entry in `provenance.exception_refs`. |

**Source**: Defined in spec `002-capability-contracts` and implemented as `enum HostApiAccess` in `crates/traverse-contracts/src/lib.rs`.

### `network_access`

Controls whether the WASM module may open outbound network connections.

| Value       | Description                                                            | Runtime enforcement |
|-------------|------------------------------------------------------------------------|---------------------|
| `forbidden` | No outbound network calls. Expected for portability-first capabilities. | Documentation-only in v0.1. The runtime does not apply a WASI network sandbox automatically. |
| `required`  | Outbound network calls are required for correct behavior.               | Documentation-only in v0.1. Authors must justify this in the capability description and governance material. |

**Source**: Defined in spec `002-capability-contracts` and implemented as `enum NetworkAccess` in `crates/traverse-contracts/src/lib.rs`.

### `filesystem_access`

Controls whether the WASM module may access the host filesystem.

| Value          | Description                                                                               | Runtime enforcement |
|----------------|-------------------------------------------------------------------------------------------|---------------------|
| `none`         | No filesystem access. Fully portable with no host filesystem assumptions.                  | Documentation-only in v0.1. The runtime does not pre-open directories or restrict filesystem WASI imports automatically. |
| `sandbox_only` | Filesystem access is allowed only within a sandbox directory provided by the host runtime. | Documentation-only in v0.1. The sandbox directory policy is defined by the host environment, not the contract itself. |

**Source**: Defined in spec `002-capability-contracts` and implemented as `enum FilesystemAccess` in `crates/traverse-contracts/src/lib.rs`.

## Authoring Steps (Create → Validate → Register)

1. Choose `namespace`, `name`, and compute `id = namespace.name`.
2. Start with `lifecycle: draft`.
3. Define strict `inputs.schema` and `outputs.schema` (avoid permissive `additionalProperties` unless you truly need it).
4. Fill in `preconditions`, `postconditions`, and `side_effects` so the full boundary is explicit.
5. Set `execution.binary_format: wasm`, `execution.entrypoint.kind: wasi-command`, and `command: run`.
6. Choose `execution.preferred_targets` (at minimum `["local"]`) and set all three constraint fields.
7. Validate locally:

```bash
cargo test -p traverse-contracts
```

8. Add the contract to a bundle manifest and inspect the bundle:

```bash
cargo run -p traverse-cli-rs -- bundle inspect <path-to-manifest.json>
```

9. Register the bundle:

```bash
cargo run -p traverse-cli-rs -- bundle register <path-to-manifest.json>
```

## Common Mistakes

- Leaving schemas permissive (for example, `additionalProperties: true`) and then expecting deterministic validation and stable tool behavior.
- Declaring side effects implicitly but forgetting to declare `side_effects` and event edges (`emits` / `consumes`).
- Using `host_api_access: exception_required` without adding an exception reference in `provenance.exception_refs`.
- Treating `preconditions` / `postconditions` as executable policy. They are documentation, not runtime code.

## Validation

For doc-only PRs, validate the repo state with:

```bash
bash scripts/ci/repository_checks.sh
```

For contract changes, also run:

```bash
cargo test -p traverse-contracts
```

## Related Documents

- [`specs/002-capability-contracts/spec.md`](../specs/002-capability-contracts/spec.md)
- [`specs/002-capability-contracts/data-model.md`](../specs/002-capability-contracts/data-model.md)
- [`docs/wasm-io-contract.md`](wasm-io-contract.md)
- [`docs/wasm-agent-authoring-guide.md`](wasm-agent-authoring-guide.md)
- [`docs/wasm-microservice-authoring-guide.md`](wasm-microservice-authoring-guide.md)

---

## Authoring a Capability Contract From Scratch (#286)

### Minimal working template

The following is the smallest valid `contract.json` you can author. Every field is required unless marked optional.

```json
{
  "kind": "capability_contract",
  "schema_version": "1.0.0",
  "id": "examples.hello-world.say-hello",
  "namespace": "examples.hello-world",
  "name": "say-hello",
  "version": "0.1.0",
  "lifecycle": "active",
  "service_type": "stateless",
  "artifact_type": "native",
  "description": "Greets a named subject and returns the greeting string.",
  "input_schema": {
    "type": "object",
    "required": ["subject"],
    "properties": {
      "subject": { "type": "string", "description": "Name to greet" }
    }
  },
  "output_schema": {
    "type": "object",
    "required": ["greeting"],
    "properties": {
      "greeting": { "type": "string", "description": "The greeting message" }
    }
  },
  "execution": {
    "binary_format": "wasm",
    "entrypoint": { "kind": "wasi-command", "command": "run" },
    "preferred_targets": ["local"],
    "constraints": {
      "host_api_access": "none",
      "network_access": "forbidden",
      "filesystem_access": "none"
    }
  },
  "provenance": {
    "spec_refs": ["002-capability-contracts"],
    "exception_refs": []
  }
}
```

### Field-by-field explanation

| Field | Required | Description |
|-------|----------|-------------|
| `kind` | Yes | Always `"capability_contract"` |
| `schema_version` | Yes | Always `"1.0.0"` in v0.x |
| `id` | Yes | Must equal `namespace + "." + name` |
| `namespace` | Yes | Dot-separated domain path (e.g. `"examples.hello-world"`) |
| `name` | Yes | Short identifier within the namespace |
| `version` | Yes | Semver `MAJOR.MINOR.PATCH` — immutable once registered |
| `lifecycle` | Yes | Start with `"draft"` until ready, then `"active"` (see Lifecycle section) |
| `service_type` | Yes | See service_type reference below |
| `artifact_type` | Yes | `"native"` for WASM binaries, `"wasm"` for explicit WASM-only |
| `description` | Yes | Human-readable summary of what the capability does |
| `input_schema` | Yes | JSON Schema object describing the input payload |
| `output_schema` | Yes | JSON Schema object describing the output payload |
| `execution` | Yes | Binary format, entrypoint, targets, constraints |
| `provenance.spec_refs` | Yes | Must include `"002-capability-contracts"` |
| `provenance.exception_refs` | Yes | Empty array unless `host_api_access: exception_required` |

### Optional fields

| Field | Description |
|-------|-------------|
| `emits` | Array of event contract IDs this capability may publish at runtime |
| `consumes` | Array of event contract IDs this capability subscribes to |
| `preconditions` | Documentation-only assertions that must hold before invocation (not enforced) |
| `postconditions` | Documentation-only assertions that must hold after invocation (not enforced) |
| `side_effects` | Description of observable side effects beyond the output schema |

### `emits` and `consumes` — connecting to event contracts

The `emits` array declares which events this capability may publish via `broker.publish()` at runtime. The runtime validates that the emitted event type is declared here; publishing an undeclared event type causes an `EventError::PolicyViolation`.

```json
"emits": ["examples.hello-world.greeted"],
"consumes": []
```

See [`docs/event-contract-authoring-guide.md`](event-contract-authoring-guide.md) for how to define the event contract itself.

### `preconditions` and `postconditions` — documentation only

These fields hold assertions about the state of the world before and after capability execution:

```json
"preconditions": [
  { "id": "pre-001", "description": "Subject name must be non-empty" }
],
"postconditions": [
  { "id": "post-001", "description": "Greeting string is non-empty and contains the subject name" }
]
```

**These are not enforced by the runtime in v0.x.** The runtime does not evaluate preconditions before execution or postconditions after. They are purely documentation — useful for human review, spec coverage, and future tooling. State this explicitly to consumers of your contract.

---

## `service_type` Reference (#295)

| Value | Meaning | Runtime implications |
|-------|---------|---------------------|
| `"stateless"` | Each invocation is independent; no state persists between calls | Runtime may freely re-invoke in any order; no session affinity required |
| `"stateful"` | The capability maintains internal state across invocations | Runtime must respect session affinity if applicable; state management is the author's responsibility |
| `"idempotent"` | Repeated invocation with identical inputs produces identical outputs with no side effects | Runtime may safely retry on transient failure |

**All expedition and hello-world examples use `"stateless"`**, which is the correct value for pure-computation WASM capabilities that receive all state through their input JSON.

Use `"stateful"` only for capabilities that explicitly manage a persistent resource (e.g. a database connection, file handle, or accumulated session). Document the state lifecycle in `description`.

---

## Validate Before Registering (#298)

Before opening a PR or registering a contract, run the spec-alignment gate against your contract:

```bash
# From repo root
bash scripts/ci/spec_alignment_check.sh

# Full repository check (includes contract schema validation)
bash scripts/ci/repository_checks.sh
```

If you want to validate a single contract file's JSON structure locally:

```bash
cargo run -p traverse-cli-rs -- bundle inspect contracts/path/to/your/bundle/manifest.json
```

The bundle inspect command will surface any structural issues (missing required fields, unknown values) before you attempt registration.

Common validation errors and fixes:

| Error | Fix |
|-------|-----|
| `missing required field 'service_type'` | Add `"service_type": "stateless"` to the contract root |
| `missing required field 'artifact_type'` | Add `"artifact_type": "native"` to the contract root |
| `host_api_access: exception_required but no exception_refs` | Add at least one entry to `provenance.exception_refs` |
| `id does not match namespace.name` | Set `id` to exactly `namespace + "." + name` |
