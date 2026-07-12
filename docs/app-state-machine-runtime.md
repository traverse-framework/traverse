# Runtime State-Machine Transitions

When an invoked capability succeeds, Traverse evaluates transitions with
`on: capability_succeeded` against the capability result under the `output`
root. Conditional transitions are evaluated in manifest order; an
unconditional transition for the same event is a fallback and is evaluated
only after all conditional transitions.

The first matching transition changes the session state. If no conditional or
fallback transition matches, the runtime emits an `error` event with
`error.code` set to `no_matching_transition` and retains the invoking state.
If an operator cannot be evaluated because the output value and condition
value have incompatible types, it emits `error.code` set to
`condition_type_error` and likewise retains the invoking state.

These events retain the capability output and execution identifiers so an app
client can diagnose the result without reimplementing routing logic.
