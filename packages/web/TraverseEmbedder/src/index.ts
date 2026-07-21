/**
 * Public Traverse platform embedder SDK for Web/TypeScript clients.
 *
 * This package is the Web row of spec `068-public-platform-embedder-packages`:
 * a versioned public package implementing the `embedder-api/1.0.0` operations
 * (spec `057-embeddable-runtime-host`) against an application-owned bundle,
 * with no production dependency on `traverse-cli serve`.
 *
 * The event envelope, deterministic identifier scheme (`sess-*`, `req-*`,
 * `evt-*`, `inst-*`), stable error codes, compatible-capability lifecycle,
 * and shutdown semantics are identical to the Rust `traverse-embedder`
 * package so every platform observes the same boundary (spec 057 FR-003).
 *
 * `BundleEmbedder` is the production implementation: it loads an
 * application-owned bundle, digest-verifies and host-ABI-validates every
 * bundled WASM capability, and executes them directly in the browser's
 * native WebAssembly host via a minimal WASI shim — no nested WASM engine,
 * no sidecar (spec 068 FR-002, NFR-001). `EmbedderTestDouble` is the
 * deterministic in-memory implementation required by spec 068 FR-006.
 */

export {
  EMBEDDER_API_VERSION,
  EMBEDDER_CONFORMANCE_VERSION,
  EMBEDDED_TRACE_API_VERSION,
  EMBEDDED_TRACE_MAX_PAGE_SIZE,
  EMBEDDED_TRACE_RETENTION_LIMIT,
  SUPPORTED_BUNDLE_SCHEMA_VERSIONS,
} from "./types.js";
export type {
  CompatibleLifecycleOutcome,
  CompatibleStartOutcome,
  EmbedderError,
  EmbedderErrorCode,
  EmbeddedTraceApi,
  EmbeddedTraceApiError,
  EmbeddedTraceApiErrorCode,
  EmbeddedTraceDetail,
  EmbeddedTraceOutcome,
  EmbeddedTracePage,
  EmbeddedTracePhase,
  EmbeddedTracePlacement,
  EmbeddedTraceSelectedTarget,
  EmbeddedTraceSummary,
  EmbedderEvent,
  EventCallback,
  JsonValue,
  ShutdownOutcome,
  SubmitOutcome,
  TraverseEmbedderApi,
} from "./types.js";

export { EmbedderTestDouble } from "./testDouble.js";
export type { EmbedderTestDoubleConfig } from "./testDouble.js";

export {
  BundleRejectedError,
  validateBundleCompatibility,
  verifyArtifactDigest,
} from "./bundleValidation.js";
export type { BundleCompatibility, BundleComponentSummary, BundleWorkflowSummary } from "./bundleValidation.js";

export { BundleEmbedder } from "./bundleEmbedder.js";
export type { BundleEmbedderConfig } from "./bundleEmbedder.js";

export { FetchBundleLoader, NodeFsBundleLoader } from "./bundleLoader.js";
export type { BundleLoader } from "./bundleLoader.js";

export {
  HOST_ABI_V1_WHITELIST,
  SUPPORTED_HOST_ABI_VERSION,
  findUnauthorizedImport,
} from "./hostAbi.js";
export type { HostAbiImport } from "./hostAbi.js";
