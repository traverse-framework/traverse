# Migrating from core-Wasm Host ABI v1 to `component-wit-v1`

Governed by spec
[`135-component-model-wit-host-capabilities`](../specs/135-component-model-wit-host-capabilities/spec.md)
and [ADR-0068](adr/0068-component-model-wit-host-capabilities.md).

Existing capabilities stay on **`core-wasm-v1`**. Host ABI v1 and its WASI /
`traverse_host` whitelist do not change. `component-wit-v1` is a separate
profile for a portable Wasm Component that must import a narrowly scoped,
versioned WIT interface implemented by an explicitly activated target-local
host.

## When to migrate

Repackage as a Component only when the capability needs an unavoidable
target-local authority (for example foreground recording). Do not use WIT as a
general plug-in, filesystem, provider, or workflow escape hatch.

## What the package must declare

```json
{
  "execution_profile": "component-wit-v1",
  "required_wit_imports": [{
    "package": "traverse:platform",
    "interface": "recording-host",
    "version": "0.1.0"
  }],
  "wit_bindings": {
    "traverse:platform/recording-host@0.1.0": "default-local-recording"
  }
}
```

The component type metadata and the manifest declaration must match exactly.
Semver ranges, package aliases, and name-only matching fail closed.

## Callweave identity

`callweave:recording/recording-host@0.1.0` is input to standardization, not an
automatic Traverse-standard alias. Keep that package during a downstream
transition only until an explicit adapter or republished
`traverse:platform/recording-host` is approved.

## Activation

A registry-published host registration never activates itself. The application
or embedder must select a trusted binding and a precise target family. Official
verified hosts require conformance evidence. Application-local fakes are labeled
`application_local_unverified` and are never a portable default.

## Verification

```bash
cargo test -p traverse-runtime --lib component_wit
```
