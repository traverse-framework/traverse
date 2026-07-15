# Changelog

## Unreleased

- Completed the `traverse-embedder-web` runtime-WASM execution path:
  `BundleEmbedder` loads an application bundle, digest-verifies and
  Traverse-Host-ABI-validates every bundled WASM capability, and executes it
  directly in the browser's native WebAssembly host through a minimal WASI
  `preview1` shim — no nested engine, no `traverse-cli serve` sidecar.
  Supports linear `direct`-triggered workflow pipelines. Verified with real
  WAT-compiled WASI test fixtures (via `wabt`), against the real checked-in
  `traverse-starter` bundle, and with a working React integration example
  (spec 068 FR-002, FR-009, NFR-001).
- Added the `traverse-embedder-web` TypeScript package: the Web embedder SDK
  boundary implementing the `embedder-api/1.0.0` surface with the shared
  event envelope and deterministic identifiers, a deterministic test double,
  bundle compatibility validation with WebCrypto digest verification, and a
  CI-enforced node test suite (spec 068).
- Added the deterministic `doc-approval.recommend` capability and canonical
  `doc-approval.pipeline` workflow (`analyze` then `recommend`, no extract
  step), completing the spec 069 pipeline with contract, agent package,
  component manifest, bundle wiring, and runtime integration tests; removed
  the superseded extract-based pipeline drafts.

- Added the public `traverse-embedder` crate: the Rust embedder SDK for Linux
  GTK and CLI clients implementing every `embedder-api/1.0.0` operation
  (init/shutdown/submit/subscribe and compatible start/stop/kill) against an
  application-owned bundle, with a deterministic test double, release
  evidence, and a CI-enforced shared conformance suite (spec 068).
- Application bundle registration now records compatible-mode components for
  traceability without registering them as runtime capabilities; their
  lifecycle is embedder-owned (spec 057).
- Added optional bearer-token authentication for MCP stdio execution commands
  and redacted full runtime traces from MCP execution/report responses by
  default.
- Added a no-default-features `traverse-runtime` core build for
  `wasm32-unknown-unknown` by feature-gating native Wasmtime and Rayon
  execution adapters.
- Added a bounded checksum-keyed `WasmExecutor` compiled-module cache while
  preserving fresh per-invocation stores and checksum/ABI validation.
- Hardened `WasmExecutor` with per-invocation fuel and store resource limits so
  runaway WASM guests are classified as timeout or resource-exhausted failures.
