# Host-authority conformance fixture: `traverse.audio-input`

Implementation-independent conformance data for the first authority under
`specs/140-host-authority-wit-adapters` (FR-013). It defines, for the WIT
adapter interface `traverse:audio-input@1.0.0`
(`contracts/connectors/traverse.audio-input/wit/capture.wit`) and the Spec 137
public envelope, what a correct dispatcher and adapter must do.

`fixture.json` is data only. It adds no runtime, adapter, or driver behavior.

## How a runner uses it

For each case, in order:

1. Build the binding described by `binding_setup` (see `binding_setups`).
2. Pick target families from `target_role`: `activated` runs the case once per
   family the embedder under test has activated an adapter for; `unadapted`
   uses a family with no adapter.
3. Script an adapter test double with each step's `adapter_calls[].returns`
   (`ok` value or `err` WIT `failure`).
4. Submit `steps[].command` (substituting `target_family`) and assert
   `expected`: `result_class`, `error_code`, `artifact_ref`, `permission_state`,
   and the ordered `events`, comparing only `compared_event_fields`.
5. Assert the adapter received exactly the listed `adapter_calls`; an empty
   list means it must not be invoked.

`expected.same_as_step` means the result equals that earlier step's result
(idempotent replay).

## Cross-target equivalence

For every `activated` case, ordered events and result fields must be identical
across all activated families once `target_family` is removed
(`cross_target_equivalence`). This is what makes "same commands, same ordered
events" checkable per Spec 140 FR-013.

## Coverage of Spec 140 FR-013

`fr_013_categories` lists the required categories; every category is tagged on
at least one case's `fr_013` list. `cargo test -p traverse-contracts --test
host_adapter_wit` enforces this.

## Redaction

`redaction.forbidden_public_substrings` must not appear in any public event,
result, error message, or evidence. The `redaction_*` cases inject
host-private diagnostics and a non-opaque `artifact-ref` to prove they do not
cross the public contract.

## Consumers

| Consumer | Ticket |
| --- | --- |
| Rust contract tests (`traverse-contracts`) | this fixture's structural validation |
| Runtime dispatch and embedder suites (Spec 057/529) | tracked in their own tickets |
