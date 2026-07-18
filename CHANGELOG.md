# Changelog

## Unreleased

- Kotlin and Swift embedder SDKs: marshal native bridge requests and map
  typed runtime bridge results, with a CI-enforced embedder conformance
  suite for both platforms (specs 071, 072).
- Kotlin embedder SDK: bound runtime execution against the Chicory Wasm
  runtime.
- Hardened durable event revocation to await completion instead of firing
  and forgetting, with test coverage for revocation failure paths.
- Recorded the decision to retain the durable event journal after
  operational evaluation (see `docs/decision-log.md` and
  `docs/adr/durable-journal-after-operational-evaluation.md`).

## v0.8.0 — 2026-07-16

See [docs/releases/v0.8.0.md](docs/releases/v0.8.0.md) for full release notes.
Highlights: public `traverse-embedder` Rust SDK and `traverse-embedder-web`
TypeScript SDK with real bundle execution (spec 068); the durable event
journal, including journal-backed replay through `subscribe`/`poll`; the
deterministic `doc-approval.recommend` capability and canonical
`doc-approval.pipeline` workflow (spec 069); HTTP API connection timeout
hardening, Sigstore placeholder-evidence rejection, and approved-specs-based
governed-artifact classification; and idempotency fixes across the event and
capability registries.
