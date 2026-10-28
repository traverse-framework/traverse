# Canonical guest input conformance (Spec 1722 FR-006)

Cross-host evidence for Spec `1722-canonical-guest-input-bytes` (Decision 118). Every host
must feed a capability guest the RFC 8785 (JCS) canonical bytes of its JSON input, and must
reject input that JCS cannot represent with `input_not_canonicalizable` before the guest
runs.

- `stdin-echo.wat`: the probe guest. It returns the exact stdin bytes it read as the JSON
  string `{"stdin":"<bytes>"}`, so a host can compare them after its own JSON round trip of
  the output.
- `stdin-echo.wasm`: the compiled guest. `canonical_input_conformance.rs` asserts it equals
  `wat::parse_file(stdin-echo.wat)`.
- `vectors.json`: the shared vectors. Each has an `input` (the caller's raw JSON text) and
  either `expected_stdin` plus `expected_sha256`, or `expected_error`. It is generated from
  the Rust reference (`traverse_contracts::canonicalize_json_bytes`) and never hand-edited;
  the Rust test fails if the reference and the file disagree.

`bytes_only: true` marks a vector that only a byte boundary can express. For example,
duplicate member names cannot survive a JS `JSON.parse`. A host whose API takes an
already-parsed value skips these.

Every input is a JSON object, because `runtime.wasm` accepts only an object as the submit
body.

## How a host runs it

For each vector, on a fresh runtime with `stdin-echo.wasm` registered as a stateless
capability:

1. Submit the vector's `input` bytes (or, for a value API, the parsed value).
2. If `expected_error` is set, the call must fail with a message containing that code.
3. Otherwise the output's `stdin` string must equal `expected_stdin`, and its UTF-8 SHA-256
   must equal `expected_sha256`.

## Where each host runs it

| Host | Test |
|---|---|
| Native runtime (wasmtime), CLI, MCP | `crates/traverse-runtime/tests/canonical_input_conformance.rs` |
| `runtime.wasm` (wasmi), as the Swift, Kotlin and .NET hosts run it | the same file, `runtime_wasm_feeds_canonical_stdin` |
| `traverse-embedder-web` (Node) | `packages/web/TraverseEmbedder/tests/canonicalInput.test.mjs` |
