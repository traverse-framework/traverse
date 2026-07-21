# ADR-0017: Govern the Local DataStore as an Embedder-Owned Durable Surface

- Status: Proposed
- Date: 2026-07-21
- Governing spec: Draft `518-durable-local-datastore`
- Owner: Traverse maintainers

## Context

`LocalFileDataStore` is the only non-test DataStore adapter, but no shipped runtime or CLI flow constructs it. Its files are plain JSON records written directly to their final paths, so a partial write or changed payload has no integrity boundary. The approved-spec registry lists Spec 032 while that source file remains Draft and also promises broader adapters that are not present.

Automatically wiring a local root into generic capability execution would add an undeclared retention, privacy, ownership, and cleanup policy. Accepting old plain files as though they were verified would create a false integrity claim.

## Decision

Treat the local DataStore as an explicitly constructed, embedder-owned library surface. The embedding application chooses the root and owns retention, backup, and deletion. Generic runtime execution does not create or select a DataStore automatically.

New local records use a versioned integrity envelope containing the record and a SHA-256 digest of its canonical serialized content. Reads verify the format, structure, and digest before returning a value. Missing, unknown, malformed, or mismatched integrity information produces the stable `integrity_check_failed` failure with a machine-readable reason.

Writes use a temporary sibling and an atomic same-directory commit only after the temporary content has been durably flushed. A failed write leaves the prior committed record visible; temporary files are never listed as state. Legacy plain record files are rejected as `legacy_unverified` and must be recreated from an authoritative application source. The public trait, capability state schema, Lamport clock, and merge semantics remain unchanged.

## Consequences

- Embedders gain a trustworthy local durable option without a hidden process-wide persistence policy.
- Corruption and interrupted writes fail predictably instead of being silently accepted or partially observed.
- Early consumers with plain files have an explicit recreate-and-rewrite migration path rather than an unsafe automatic conversion.
- SQLite, browser, cloud, replication, and runtime-default storage remain separate decisions; this ADR does not imply their delivery.

## Alternatives Considered

- Automatically instantiate the local adapter for every runtime: rejected because root ownership, retention, and application isolation are undefined.
- Continue accepting plain records: rejected because parsing is not integrity verification and would falsely trust altered state.
- Silently migrate old files: rejected because there is no trustworthy source from which to derive a verification digest after the fact.
- Add SQLite, browser, and cloud adapters now: rejected as a broader portability and operational design than the isolated local durability gap.
