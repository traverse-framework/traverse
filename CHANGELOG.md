# Changelog

## Unreleased

- Added a bounded checksum-keyed `WasmExecutor` compiled-module cache while
  preserving fresh per-invocation stores and checksum/ABI validation.
- Hardened `WasmExecutor` with per-invocation fuel and store resource limits so
  runaway WASM guests are classified as timeout or resource-exhausted failures.
