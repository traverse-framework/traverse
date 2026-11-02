# Feature Specification: Suspended Connector Invocation

**Status**: Approved (2026-11-02)
**Canonical governing ID**: `1719-suspended-connector-invocation`
**Version**: 1.0.0
**Extends**: `104-mediated-connector-invocation`,
`1402-runtime-wasm-orchestrator-convergence`, and
`071-native-runtime-wasm-bridge`.
**Decision evidence**: Decision 122 in Traverse #1719 (2026-11-02).

## Purpose and boundary

Let a host answer a guest's synchronous `traverse_host.connector_invoke`
asynchronously when the guest runs inside `runtime.wasm`. Spec 104 still governs
what a connector call is allowed to do. This spec governs only how the answer
reaches a suspended guest.

Native Wasmtime hosts answer in the call and aren't affected.

## Requirements

- **FR-001**: A host MAY name connectors it answers asynchronously in the
  `traverse_init` header as `deferred_connectors`, a list of
  `{connector_id, version}`. An entry activates that connector for this
  instance. A host that names none MUST see no change in behavior.
- **FR-002**: A call to a deferred connector MUST pass the same Spec 104 checks
  as any other call, in the same order, before the guest suspends: request
  bounds, declared, activated and semver-compatible. A call that fails them MUST
  return its Spec 104 failure code to the guest without suspending.
- **FR-003**: A call that passes MUST suspend the guest at that call.
  `traverse_submit` MUST then return `status: "suspended"` and
  `pending_connector_invocation`, the guest's request.
- **FR-004**: While a guest is suspended, `traverse_submit` MUST treat its
  payload as the answer: `{"kind": "connector_response", "response":
  <ConnectorInvokeResponse>}`, or `{"kind": "connector_response", "failure":
  <message>}` when the host couldn't answer. It MUST NOT start other work.
- **FR-005**: An answer MUST pass the same Spec 104 checks as an in-call answer:
  ABI version, non-empty result class, host-private output filter and 64 KiB
  bound. An answer that fails them, or a payload that can't be parsed, MUST reach
  the guest as the matching Spec 104 failure code, and the guest then continues.
- **FR-006**: The guest MUST resume at the suspended call with its memory, fuel
  remaining and events intact. Time spent suspended MUST NOT consume fuel. A
  resumed guest MAY suspend again.
- **FR-007**: Every suspended call MUST add exactly one
  `connector_invocation_evidence` entry, as an in-call answer does.
- **FR-008**: `traverse_shutdown` MUST drop a suspended guest.
- **FR-009**: The exported symbol set and the bridge ABI version MUST NOT change.
- **FR-010**: A deferred connector MUST fail closed as `unbound` on a path that
  can't suspend, including a nested capability invoked by a Spec 139 app state
  machine.
- **FR-011**: `traverse-embedder-web` MUST expose
  `BundleEmbedder.activateMediatedConnector(connectorId, version, connector)`
  and a first-party `traverse.model-runtime` binding over
  `ExactModelBrowserHost`, with responses the same shape as the native binding.
  A capability target MUST take its requirements from its contract's
  `connector_requirements`.
- **FR-012**: `submitAsync` MUST drive a suspended guest to its result. Sync
  `submit` can't wait for an answer, so it MUST fail a target that has an active
  deferred connector with `async_submit_required`.
- **FR-013**: Cancelling `submitAsync` while a guest is suspended MUST drop the
  guest and report the session as cancelled.

## Scenarios

1. A guest calling a deferred connector suspends, the host answers, and the guest
   finishes with that answer in its output and one evidence entry.
2. A guest making two deferred calls suspends twice and finishes.
3. A guest calling an undeclared connector gets `undeclared` without suspending.
4. An answer holding host-private data reaches the guest as a failure code, with
   `unauthorized_output` evidence.
5. Without `deferred_connectors`, the same guest fails closed as `unbound`.
6. Browser and Node: a target running a guest against
   `traverse.model-runtime` returns the same response shape as the native binding
   for `usage_undeclared` and for a connector that isn't activated.

## Out of scope

- The Swift, Kotlin and .NET activation APIs. Their READMEs document this as not
  yet shipped.
