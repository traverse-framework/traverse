# traverse-embedder-web

Public Traverse platform embedder SDK for Web/TypeScript clients — the Web
row of spec `068-public-platform-embedder-packages`, exposing the
[`embedder-api/1.0.0`](../../../specs/057-embeddable-runtime-host/embedder-api-1.0.0.json)
operation surface. Production execution requires no `traverse-cli serve`
sidecar and no `.traverse/server.json` discovery.

## Runtime-WASM execution

Unlike the native platform embedders, which embed Wasmtime to execute
bundled capability artifacts, the browser *is already* a WebAssembly host.
`BundleEmbedder` compiles each bundled capability module with
`WebAssembly.compile` once at `init`, validates its imports against the
Traverse Host ABI whitelist (only `wasi_snapshot_preview1.fd_read` /
`fd_write` / `proc_exit` and the reserved `traverse_host.*` imports — no
filesystem, no network, no environment, deny-by-default, matching the native
`WasmExecutor`), and instantiates + invokes it synchronously per `submit`
through a minimal WASI `preview1` shim that pipes JSON stdin/stdout exactly
like the native executor. Workflow execution supports linear,
`direct`-triggered pipelines (the shape used by every bundled example
workflow today); event-driven/conditional edges are rejected deterministically
at `init` rather than silently mis-executed.

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

`npm test` compiles a set of real WASI capability modules from WebAssembly
Text format via `wabt` (a devDependency, mirroring the Rust crate's `wat`
crate test fixtures) and runs `BundleEmbedder` against them — including one
test that loads and executes the real, checked-in `apps/traverse-starter`
bundle end to end — so the browser execution engine is exercised for real,
not mocked.
