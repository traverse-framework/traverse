# Next Release Notes

Unreleased changes since `v0.14.0` will be recorded here.

## Breaking: model rights enforcement (Spec 138 0.8.0, Decision 107, `#1599`)

- **`model_usage` is required.** Apps with `exact_model_dependencies` must
  declare `model_usage: commercial | non_commercial`; without it,
  registration and execution fail closed with
  `model_incompatible` / `usage_undeclared`. Set it on
  `ExactModelHostConnector::model_usage` (Rust). In Swift, pass
  `ExactModelHost(pins:trustedPublicKeysHex:modelUsage:limits:)`; the new
  `modelUsage` argument has no default, so existing call sites must be
  updated.
- **Usage policy:** a `commercial` usage (from the app, or forced by the
  host's `host_requires_commercial`) rejects `prohibited` packages with
  `rights_policy_denied`.
- **New reasons:** `usage_undeclared`, `rights_policy_denied`,
  `rights_inconsistent` and `package_revoked`. Rights failures carry a
  structured `error.detail`, and the Spec 137 dispatch failure now preserves
  the adapter's `reason` and `detail`.
- **Package status:** a host-owned status map via
  `ExactModelHostConnector::set_package_status`. `revoked` blocks the next
  register or execute; `deprecated` runs but is flagged.
- **Rights in evidence:** every `model.execute` result and dispatch document
  carries `model_evidence`, the verified rights record. The
  `traverse.model-runtime` connector contract is now `2.1.0` (additive).
- **Manifest schema `2.1.0`:** adds an optional `rights.derivation`; hosts
  accept `2.0.0` and `2.1.0`.
- **Shared rights conformance suite:**
  `fixtures/models/rights-conformance/suite.json`. Web (`#1600`) and Swift
  (`#1601`) parity follow.
