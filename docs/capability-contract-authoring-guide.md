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

## Contract surface coverage (honesty)

Treat `use_cases` as the executable promise for the **entire declared schema
surface** (Spec `102` v1.1.0 / Decision 58) — not a minimum example count:

- Every string `enum` under `inputs.schema` MUST appear in ≥1
  `use_cases[].input_example` at the same path.
- Every top-level `inputs.schema.required` property MUST appear in ≥1
  `use_cases[].input_example`.
- `outputs.schema.properties.reason_code` and `status`, when used for
  checkable outcomes, MUST be enums; every enum value MUST appear in ≥1
  `use_cases[].output_example`.
- `use_cases` MUST be non-empty; `capability publish` preserves them into the
  registry record.
- Each use case MUST have a matching package smoke fixture
  (`runtime-requests/ucNN-*.json`) that asserts its `reason_code` / key outputs.

Do not list enum values the artifact only rejects with a generic
`unsupported_*` stub unless that failure is itself a documented use case.
Description prose beyond the use-case matrix MUST be removed or called out
under **Known limitations**. Narrowing an overclaimed surface via an honesty
patch-bump is a valid fix.

Governed by Spec `102-contract-surface-coverage` / ADR-0038.
`capability publish` / `--dry-run` and registry CI enforce the gate
(issues #1040 / registry#215).

## Persona references

When a use case names `persona_ref`, that id MUST already exist in the target
registry checkout as `personas/<id>/<version>/persona.json`.
`capability publish` / `--dry-run` resolve every referenced persona against the
`--registry-repo` tree and fail with `capability_publish_persona_ref_unresolved`
before opening a registry PR (issue #1011).

## Common Mistakes

- Leaving schemas permissive (for example, `additionalProperties: true`) and then expecting deterministic validation and stable tool behavior.
- Advertising enum values or description features that have no covering use case (see Contract surface coverage above; incident: `core.process-comment@1.0.0`).
- Referencing a `persona_ref` that is not yet published under `personas/<id>/` in the registry checkout.
- Declaring side effects implicitly but forgetting to declare `side_effects` and event edges (`emits` / `consumes`).
- Using `host_api_access: exception_required` without adding an exception reference in `provenance.exception_refs`.
- Treating `preconditions` / `postconditions` as executable policy. They are documentation, not runtime code.
- Putting a guest-denied field in JSON Schema `required`, then wondering why smoke never sees the guest `reason_code` (see Guest-enforced fields above).

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

The `emits` array declares which events this capability may publish at runtime via the `traverse_host::emit_event` WASM host function (spec `098-capability-event-host-abi`). The host validates the emitted `event_id`/`version` against this list synchronously, at call time — an undeclared emission is rejected immediately, before execution completes, not discovered afterward. Only capabilities whose `service_type` is `Subscribable` may call the host function at all; any other `service_type` is rejected regardless of payload.

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

### Guest-enforced fields (host schema vs guest denials)

Host JSON Schema validation runs before the WASM guest. If a field is listed in
JSON Schema `required`, unhappy-path use cases that omit it never reach the
guest — the host rejects them first — so the guest cannot return a structured
`reason_code` for that precondition.

When a denial must be guest-authored (for example `reason_code:
invalid_principal`), keep that field **schema-optional** and enforce it in the
guest instead. Document the choice so it is not mistaken for accidental
weakening:

1. Omit the field from the nearest JSON Schema `required` array.
2. State in the property (or parent object) `description` that the guest
   enforces the field and name the covering use case / `reason_code`.
3. Cover the omission with an explicit `use_cases[]` entry and a package smoke
   fixture that reaches the guest.

Default host validation for normal requests stays fail-closed: other required
fields remain required, and guests must still reject empty/invalid optional
values they claim to enforce. Do not use a global “skip input schema” escape
hatch for production execute paths.

Working example: `examples/core-authorize` UC-09 — `principal.id` is
guest-enforced and schema-optional; the guest returns `invalid_principal`.

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

## `risk` Reference (spec 109)

Every contract carries an immutable `risk` classification across four
independent dimensions (spec `109-runtime-workflow-proposals` FR-005, ADR-0041).
A single field never implies another dimension — declare each one deliberately.

```json
"risk": {
  "effect_class": "pure_read",
  "determinism_class": "deterministic",
  "data_flow": {
    "accepted_data_classifications": [
      { "field_path": "/comment_text", "classification": "internal" }
    ],
    "produced_data_classifications": [
      { "field_path": "/draft_id", "classification": "public" }
    ],
    "egress_policy": "denied"
  },
  "reliability": {
    "idempotency_required": false,
    "retryable": true,
    "compensation_available": false
  }
}
```

| Dimension | Values | Meaning |
|---|---|---|
| `effect_class` | `pure_read`, `state_write`, `external_effect`, `irreversible_effect` | What kind of effect invoking this capability has on the world. |
| `determinism_class` | `deterministic`, `externally_variable`, `model_derived` | Whether repeated invocation with the same inputs is guaranteed to agree. |
| `data_flow` | `accepted_data_classifications`, `produced_data_classifications` (JSON-Pointer `field_path` + `classification`: `public`/`internal`/`confidential`/`restricted`), `egress_policy` (`"denied"` or `{"allowed_connectors": [...]}`) | Field-level data classification and which connectors classified data may flow to. Schema compatibility alone never authorizes disclosure. |
| `reliability` | `idempotency_required`, `retryable`, `compensation_available` (booleans) | Reliability semantics a caller must honor. |

**Migration behavior.** Contracts published before spec 109 have no `risk`
field at all. `traverse-contracts` treats a missing `risk` as present but set
to the most conservative value on every dimension
(`irreversible_effect`/`model_derived`/all egress denied/idempotency
required) — see `traverse_contracts::default_risk_metadata()`. A pre-109
contract is therefore never silently treated as safe to run without
authorization; author it explicitly once you know the capability's real
classification. Because the field is additive with a safe default, adding
`risk` to an existing contract is a backward-compatible change under this
repo's [compatibility policy](compatibility-policy.md), the same as
`service_type` or `permitted_targets` before it. Within one contract version,
`risk` is covered by the existing immutable-publish digest check — it cannot
change without republishing under a new version.

**Automatic-eligible gate.** `traverse_contracts::is_automatic_eligible(&risk)`
is the single, canonical decision of whether a proposal built only from this
capability's declared risk classes may run without an approval token (spec 109
FR-006). Every caller that gates automatic execution — including the future
P1 proposal runtime (issue #1090) — must consume this function rather than
re-deriving the rule from individual fields.

**Manifest tightening, never weakening.** An application manifest may narrow
a component's egress surface below what its contract allows, but can never
grant egress the contract's `risk` forbids. Declare the narrower set on the
component manifest:

```json
"risk_policy": {
  "egress_allowed_connectors": ["traverse.http"]
}
```

`traverse-cli` validates this at `app register`/`app validate` time via
`traverse_contracts::validate_manifest_risk_policy`: an `egress_allowed_connectors`
entry not present in the contract's `egress_policy` allowlist (or any entry at
all when the contract's `egress_policy` is `"denied"`) fails with the stable
`risk_policy_weakened` code. `effect_class`, `determinism_class`, and
`reliability` are immutable facts about the capability's own behavior — a
manifest has no override for them at all.

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
