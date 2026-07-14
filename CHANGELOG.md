# Changelog

## Unreleased

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
