# traverse-embedder-web

Public Traverse platform embedder SDK for Web/TypeScript clients — the Web
row of spec `068-public-platform-embedder-packages`, exposing the
[`embedder-api/1.0.0`](../../../specs/057-embeddable-runtime-host/embedder-api-1.0.0.json)
operation surface. Production execution requires no `traverse-cli serve`
sidecar and no `.traverse/server.json` discovery.

## Runtime-WASM execution

`BundleEmbedder` loads the application bundle's `runtime/runtime.wasm`
orchestrator (spec `1402` / `068` FR-002 — **not** shipped inside this npm
package) via `BundleLoader`, digest-verifies it, and drives the same
`runtime-wasm-bridge/1.0.0` ABI native hosts use (`traverse_init` /
`traverse_submit` / `traverse_next_event`). Nested capability execution and
`traverse_host::emit_event` validation run inside that orchestrator (Decision
86–89), not in a hand-rolled TypeScript WASI/`emit_event` path.

App bundles must include `runtime/runtime.wasm` plus
`runtime/runtime.wasm.sha256` (`sha256:<64 hex>`). Workflow execution still
supports linear `direct`-triggered pipelines; event-driven/conditional edges
are rejected deterministically at `init`.

**Migration (Phase 3 / FR-010):** the interim TypeScript `emit_event` host
(#1404), `wasi.ts`, and per-capability Host ABI import gating are removed.
Bundles without `runtime/runtime.wasm` fail closed at `init`. This is a
breaking embedder change; the next lockstep cut that publishes it MUST be
`0.11.0` (or later), not a `0.10.x` patch.

```ts
import { BundleEmbedder, FetchBundleLoader } from "traverse-embedder-web";

const embedder = await BundleEmbedder.init({
  manifestPath: "/bundles/my-app/app.manifest.json",
  loader: new FetchBundleLoader(),
  platform: "web",
});
embedder.subscribe((event) => console.log(event));
embedder.submit("my-app.process", { note: "hello" });
```

See `examples/react-integration/` for a working React page that loads the
checked-in `traverse-starter` bundle straight from the repository and
executes it with no `traverse-cli serve` process running.

## Operations

| `embedder-api/1.0.0` | TypeScript surface |
| --- | --- |
| `runtime.submit` | `TraverseEmbedderApi.submit(targetId, input)` |
| `runtime.subscribe` | `TraverseEmbedderApi.subscribe(callback)` (ordered, replayed) |
| `runtime.shutdown` | `TraverseEmbedderApi.shutdown()` |
| `compatible.start` | `TraverseEmbedderApi.startCompatible(capabilityId, input)` |
| `compatible.stop` | `TraverseEmbedderApi.stopCompatible(capabilityId, instanceId?)` |
| `compatible.kill` | `TraverseEmbedderApi.killCompatible(capabilityId, instanceId?)` |

Events are JSON values with the same envelope as every Traverse embedder
package (`kind: "embedder_event"`, `event_id`, `sequence`, `event_type`,
`workspace_id`, `app_id`, `session_id`, `data`) and the same deterministic
identifier scheme (`sess-*`, `req-*`, `evt-*`, `inst-*`), so the same
operations produce identical event JSON on every platform.

## Bundle compatibility

`validateBundleCompatibility(appManifest)` parses and deterministically
validates an application bundle manifest (spec
`044-application-bundle-manifest`): supported `schema_version` values
(`1.0.0`), component identity fields, and `sha256:` digest metadata.
`verifyArtifactDigest(bytes, declaredDigest, label)` verifies bundled
artifact bytes with WebCrypto. Incompatible bundles are rejected with stable
error codes (`unsupported_bundle_schema`, `bundle_load_failed`) and never
fall back to a sidecar (spec 068 NFR-001).

## Test double

`EmbedderTestDouble` implements `TraverseEmbedderApi` with scripted results,
the shared event envelope, deterministic identifiers, the full
compatible-capability lifecycle (including the `platforms[]` allowlist
guard), and idempotent shutdown — for host tests without WASM or network
(spec 068 FR-006):

```ts
import { EmbedderTestDouble } from "traverse-embedder-web";

const embedder = new EmbedderTestDouble({ appId: "my-app", platform: "web" })
  .withTargetOutput("my-app.process", { status: "processed" })
  .withCompatibleTarget("my-app.render", ["web"]);

embedder.subscribe((event) => console.log(event));
embedder.submit("my-app.process", { note: "hello" });
embedder.shutdown();
```

## IndexedDB DataStore

`IndexedDbDataStore` is the browser backend for the existing host-owned
DataStore port (Spec `085-datastore-indexeddb`). The embedding application
must supply an origin-scoped database name and owns its lifecycle, retention,
backup, and deletion policy. Traverse does not choose a default database.

```ts
import { IndexedDbDataStore } from "traverse-embedder-web";

const store = await IndexedDbDataStore.open({
  databaseName: "my-app-state",
  classification: "public",
});
await store.write({
  key: "draft",
  value: { status: "ready" },
  lamport_clock: 1,
  writer_id: "browser-host",
});
const draft = await store.read("draft");
store.close();
```

The adapter holds an exclusive Web Lock for its lifetime. A contender receives
`store_locked`; a browser without Web Locks receives `locking_unsupported`.
Public records use the same `local-datastore/1` SHA-256 integrity envelope as
the native backend. Quota and persistence failures are typed and writes are
never silently dropped. Private operations fail with `key_provider_required`,
and `prune`, `backup`, and `restore` fail with `unsupported`; IndexedDB v1 does
not store private plaintext or provide maintenance archives.

## Error mapping

Boundary failures use stable `EmbedderErrorCode` values —
`bundle_load_failed`, `unsupported_bundle_schema`, `runtime_stopped`,
`target_not_found`, `compatible_lifecycle_required`,
`capability_not_compatible`, `platform_not_supported`, `instance_not_found`,
`instance_not_running` — identical to the Rust `traverse-embedder` crate.
Runtime execution failures surface inside `error` events with the runtime's
stable snake_case codes. Secrets never appear in events, errors, or evidence
(spec 068 NFR-004).

## Compatibility and upgrade policy

- Embedder API `1.0.0`; a new IDL version requires a new conformance suite
  revision and a release stating the new version in its evidence.
- Supported bundle schema versions: `1.0.0`.
- Semantic versioning; the package versions in lockstep with the Traverse
  workspace.

## Release evidence

`releaseEvidence()` returns JSON recording the package name/version, the
runtime implementation (`browser-webassembly` for `BundleEmbedder`,
`test-double` for `EmbedderTestDouble`), embedder API + conformance
versions, supported bundle schemas, bundle identity, and the sha-256 digest
of every bundled WASM component (spec 068 FR-008, NFR-002).

## Development

```bash
npm install
npm test   # builds with tsc, then runs the node:test suite
```

## Publishing

Published to npm as [`traverse-embedder-web`](https://www.npmjs.com/package/traverse-embedder-web)
(currently `0.10.2`; verify with `npm view traverse-embedder-web version`):

```bash
npm install traverse-embedder-web
```

Maintainers: see
[docs/web-embedder-npm-publish-runbook.md](../../../docs/web-embedder-npm-publish-runbook.md)
for the preflight checklist and exact publish commands.

`npm test` compiles a set of real WASI capability modules from WebAssembly
Text format via `wabt` (a devDependency, mirroring the Rust crate's `wat`
crate test fixtures) and runs `BundleEmbedder` against them — including one
test that loads and executes the real, checked-in `examples/applications/traverse-starter`
bundle end to end — so the browser execution engine is exercised for real,
not mocked.
