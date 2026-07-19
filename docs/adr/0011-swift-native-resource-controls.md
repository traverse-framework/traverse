# ADR-0011: Require Supported Swift Resource Controls Before Certification

- Status: Accepted
- Date: 2026-07-18

## Context

Decision 27 selected WasmKit 0.3.1 on the assumption that it exposed public
memory and execution-resource controls. Official WasmKit 0.3.1 source shows
that `Store.resourceLimiter` remains `@_spi(Fuzzing)` and that it exposes no
supported fuel, epoch, deadline, or interruption API. Raising the Swift and
Apple platform floors therefore does not satisfy the production requirement in
Spec 071.

## Decision

Do not certify a Swift package or a cross-platform Native Embedder Baseline
until its selected runtime profile proves bounded memory growth and
deterministic interruption through documented public APIs on iOS devices and
macOS. Unsupported SPI and watchdogs that cannot stop the untrusted execution
are prohibited.

WasmKit remains an eligible candidate only if a future supported release meets
these requirements. An alternative engine requires a separate approved
decision, dependency review, Apple bundle-distribution evidence, and the full
bridge conformance corpus.

## Consequences

- Decision 27 is superseded: a WasmKit version upgrade alone is not an
  implementation path for #647.
- #647 remains blocked until #762 produces a certified profile or confirms an
  upstream dependency.
- Kotlin and .NET work may continue, but releases cannot be represented as a
  cross-platform native baseline while Swift is uncertified.

## Alternatives Considered

- Use WasmKit SPI: rejected because it is unsupported and cannot support a
  production certification claim.
- Use an external watchdog around a non-interruptible interpreter: rejected
  because it can return to the UI while untrusted code continues consuming
  resources.
- Switch engines immediately: rejected until a device-level feasibility,
  security, packaging, and conformance evaluation proves a supported profile.
