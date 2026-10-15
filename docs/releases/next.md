# Next Release Notes

Unreleased changes since `v0.15.0` will be recorded here.

## Fix: `model verify --model-usage` and JSON usage errors (`#1686`)

- `traverse-cli model verify` (and `conformance generate|check`) accept
  `--model-usage commercial|non_commercial` and optional
  `--require-model-usage`.
- Without `--model-usage`, verification still uses a non-commercial default
  but the JSON report sets `rights_record.effective_usage` to `null` and
  keeps `verification_usage` explicit (no false app declaration).
- `--model-usage commercial` on a `commercial_use: prohibited` package fails
  with `rights_policy_denied`; invalid values fail closed.
- When `--json` is present, argument/usage errors (malformed keys, unknown
  flags, bad `--model-usage`) emit a JSON error envelope on stdout with
  `error.code: "usage_error"` and a non-zero exit.


## Fix: host-configurable nested limits for `runtime.wasm` (`#1656`)

- Nested capability fuel / memory inside `runtime.wasm` stay defaulted to
  10M / 32 MiB, but hosts can raise them via `traverse_init` `limits` and
  `BundleEmbedder.init({ limits: { fuel, maxMemoryBytes } })`.
- Bundle load / native registration fail closed with
  `resource_requirements_exceed_host` when a capability's declared
  `execution.resources` or WASM initial memory exceeds the host ceiling.
- Spec 1402 FR-012 amended to 1.5.0; contract `execution.resources` is
  optional and backwards-compatible.
