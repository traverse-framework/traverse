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
 * application-owned bundle (including `runtime/runtime.wasm`),
 * digest-verifies every bundled WASM capability, and executes them through
 * the shared `runtime.wasm` orchestrator (spec `1402` FR-006).
 * `EmbedderTestDouble` is the deterministic in-memory implementation
 * required by spec 068 FR-006.
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
  AppCommandEnvelope,
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
export type {
  BundleEmbedderConfig,
  HostConnectorAdapter,
  HostConnectorAdapterResult,
} from "./bundleEmbedder.js";

export {
  BROWSER_PLAN_MAX_CANDIDATES,
  BROWSER_PLAN_MAX_DEPENDENCIES,
  BROWSER_PLAN_MAX_FACT_BYTES,
  BROWSER_PLAN_MAX_NODES,
  BROWSER_WORKFLOW_PROPOSAL_SCHEMA_VERSION,
  BrowserPlanError,
  browserLocalPlan,
  SUPPORTED_BROWSER_PLAN_CONTRACT_SCHEMA_VERSION,
} from "./browserLocalPlan.js";

export {
  COMPOSED_WORKFLOW_MAX_NODES,
  COMPOSED_WORKFLOW_MAX_PAYLOAD_BYTES,
  ComposedWorkflowError,
  executeBrowserComposedWorkflow,
} from "./composedWorkflow.js";
export type {
  ComposedCapabilityEvent,
  ComposedWorkflowErrorCode,
  ComposedWorkflowExecutionOptions,
  ComposedWorkflowNodeOutcome,
  ComposedWorkflowTrace,
} from "./composedWorkflow.js";

export {
  RuntimeWasmHost,
  RuntimeWasmHostError,
  mapRuntimeWasmEvents,
  mapServiceType,
  parseDeclaredEmits,
} from "./runtimeWasmHost.js";
export type {
  RuntimeWasmCapabilityInit,
  RuntimeWasmEmitRef,
  RuntimeWasmJson,
} from "./runtimeWasmHost.js";

export type {
  BrowserPlanErrorCode,
  BrowserPlanResponse,
  BrowserPlanTarget,
  BrowserProposalMapping,
  BrowserProposalNode,
  BrowserSnapshotIdentity,
  BrowserWorkflowProposal,
} from "./browserLocalPlan.js";

export { FetchBundleLoader, NodeFsBundleLoader } from "./bundleLoader.js";
export type { BundleLoader } from "./bundleLoader.js";

export {
  MemoryRegistryCacheStore,
  RegistryCacheError,
  evictAllRegistryCacheEntries,
  evictRegistryCacheEntry,
  prepareRegistryDependency,
  resolveRegistryDependencyOffline,
} from "./registryCache.js";
export type {
  PublicRegistryCapabilityRecord,
  RegistryArtifactFetcher,
  RegistryCacheErrorCode,
  RegistryCacheStore,
  RegistryPrepareEvidence,
  RegistryReference,
  SyncedPublicRegistryState,
  VerifiedRegistryDependency,
} from "./registryCache.js";

export { IndexedDbDataStore, IndexedDbDataStoreError } from "./indexedDbDataStore.js";
export type {
  DataClassification,
  IndexedDbDataStoreConfig,
  IndexedDbDataStoreErrorCode,
  IndexedDbDataStoreOperation,
  StateRecord,
} from "./indexedDbDataStore.js";

export {
  STATEFUL_BROWSER_STORE_UNAVAILABLE,
  attestStatefulBrowserActivation,
} from "./statefulBrowserActivation.js";
export type {
  StatefulBrowserActivationDenial,
  StatefulBrowserActivationEvidence,
  StatefulBrowserActivationReason,
  StatefulBrowserActivationResult,
} from "./statefulBrowserActivation.js";

export {
  HOST_ABI_V1_WHITELIST,
  SUPPORTED_HOST_ABI_VERSION,
} from "./hostAbi.js";
export type { HostAbiImport } from "./hostAbi.js";

export {
  AUDIO_CAPTURE_OPERATION,
  AUDIO_INPUT_CONNECTOR,
  HOST_CONNECTOR_COMMAND_KIND,
  HOST_CONNECTOR_COMMAND_SCHEMA_VERSION,
  HOST_CONNECTOR_EVENT_KIND,
  HOST_CONNECTOR_GOVERNING_SPEC,
  HOST_CONNECTOR_RESULT_KIND,
  MODEL_EXECUTE_OPERATION,
  MODEL_RUNTIME_CONNECTOR,
  MODEL_RUNTIME_GOVERNING_SPEC,
  PLACEMENT_WASM_CPU,
  audioCaptureCommand,
  modelExecuteCommand,
  normalizeModelExecuteEvidence,
} from "./hostConnectorCommand.js";
export type {
  HostConnectorAppCommand,
  HostConnectorError,
  HostConnectorErrorCode,
  HostConnectorEvent,
  HostConnectorEventName,
  HostConnectorTargetFamily,
  ModelExecutePayload,
  ModelRef,
} from "./hostConnectorCommand.js";

export { executeVerifiedEntrypoint, VerifiedEntrypointError } from "./verifiedEntrypoint.js";
export type {
  VerifiedEntrypointFetch,
  VerifiedEntrypointRequest,
  VerifiedEntrypointResponse,
} from "./verifiedEntrypoint.js";

export {
  ExactModelBrowserHost,
  ExactModelError,
  ModelIoStore,
  MODEL_EXECUTE_EXPORT,
  MODEL_GUEST_ABI_VERSION,
  encodeGuestFrame,
} from "./exactModel.js";
export type { ExactModelPin, ModelPackageManifest } from "./exactModel.js";
