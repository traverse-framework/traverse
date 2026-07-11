# Changelog

## Unreleased

- Hardened `WasmExecutor` with per-invocation fuel and store resource limits so
  runaway WASM guests are classified as timeout or resource-exhausted failures.
