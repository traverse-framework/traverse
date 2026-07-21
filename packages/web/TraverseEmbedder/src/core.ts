/**
 * Shared deterministic embedder state: identity, counters, subscribers,
 * event history, and the compatible-capability lifecycle table. Both
 * `EmbedderTestDouble` and `BundleEmbedder` delegate here so their public
 * boundary behavior — event envelope, deterministic identifiers, and
 * compatible lifecycle — is identical (mirrors the Rust SDK's `EmbedderCore`).
 */
import {
  EMBEDDER_API_VERSION,
  EMBEDDER_CONFORMANCE_VERSION,
  EMBEDDED_TRACE_API_VERSION,
  EMBEDDED_TRACE_MAX_PAGE_SIZE,
  EMBEDDED_TRACE_RETENTION_LIMIT,
  EVENT_SCHEMA_VERSION,
  PACKAGE_NAME,
  PACKAGE_VERSION,
  SUPPORTED_BUNDLE_SCHEMA_VERSIONS,
  embedderError,
  errorValue,
  paddedId,
  runtimeStoppedError,
} from "./types.js";
import type {
  CompatibleLifecycleOutcome,
  CompatibleStartOutcome,
  EmbeddedTraceApiError,
  EmbeddedTraceApiErrorCode,
  EmbeddedTraceDetail,
  EmbeddedTraceOutcome,
  EmbeddedTracePage,
  EmbeddedTracePhase,
  EmbeddedTracePlacement,
  EmbeddedTraceSelectedTarget,
  EmbedderError,
  EmbedderEvent,
  EventCallback,
  JsonValue,
  SubmitOutcome,
} from "./types.js";

let nextEmbeddedTraceSession = 1;

type InstanceState = "started" | "stopped" | "killed";

interface CompatibleInstance {
  readonly capabilityId: string;
  state: InstanceState;
}

export interface EmbeddedTraceRecordInput {
  readonly executionId: string;
  readonly targetId: string;
  readonly outcome: EmbeddedTraceOutcome;
  readonly phases: readonly EmbeddedTracePhase[];
  readonly selectedTarget: EmbeddedTraceSelectedTarget | null;
  readonly placement: EmbeddedTracePlacement | null;
  readonly failureCode: string | null;
  readonly stateMachineValid: boolean | null;
}

function logicalCompletionTime(sequence: number): string {
  const minute = Math.floor(sequence / 60) % 60;
  const second = sequence % 60;
  return `1970-01-01T00:${String(minute).padStart(2, "0")}:${String(second).padStart(2, "0")}Z`;
}

function traceError(code: EmbeddedTraceApiErrorCode): EmbeddedTraceApiError {
  switch (code) {
    case "invalid_cursor":
      return { code, message: "the trace cursor is invalid for this embedded session" };
    case "trace_not_found":
      return { code, message: "the requested trace is not retained by this embedded session" };
    case "trace_api_unavailable":
      return { code, message: "the embedded Trace API is unavailable because the host is stopped" };
    case "incompatible_version":
      return { code, message: "the requested embedded Trace API version is not supported" };
  }
}

export class EmbedderCore {
  readonly workspaceId: string;
  readonly appId: string;
  readonly appVersion: string;
  readonly platform: string;
  readonly compatibleTargets: Map<string, readonly string[]>;
  private readonly instances = new Map<string, CompatibleInstance>();
  private readonly subscribers: EventCallback[] = [];
  private readonly history: EmbedderEvent[] = [];
  private nextEvent = 0;
  private nextSession = 0;
  private nextRequest = 0;
  private nextInstance = 0;
  private readonly traceSession = nextEmbeddedTraceSession++;
  private nextTrace = 0;
  private readonly traces: EmbeddedTraceDetail[] = [];
  stopped = false;

  constructor(
    workspaceId: string,
    appId: string,
    appVersion: string,
    platform: string,
    compatibleTargets: Map<string, readonly string[]>,
  ) {
    this.workspaceId = workspaceId;
    this.appId = appId;
    this.appVersion = appVersion;
    this.platform = platform;
    this.compatibleTargets = compatibleTargets;
  }

  nextSessionId(): string {
    this.nextSession += 1;
    return paddedId("sess", this.nextSession);
  }

  nextRequestId(): string {
    this.nextRequest += 1;
    return paddedId("req", this.nextRequest);
  }

  recordTrace(input: EmbeddedTraceRecordInput): void {
    this.nextTrace += 1;
    const completionSequence = this.nextTrace;
    this.traces.push({
      summary: {
        traceId: `embedded-trace-${String(this.traceSession).padStart(8, "0")}-${String(completionSequence).padStart(8, "0")}`,
        executionId: input.executionId,
        targetId: input.targetId,
        completedAt: logicalCompletionTime(completionSequence),
        completionSequence,
        outcome: input.outcome,
      },
      phases: [...input.phases],
      selectedTarget: input.selectedTarget,
      placement: input.placement,
      failureCode: input.failureCode,
      stateMachineValid: input.stateMachineValid,
    });
    if (this.traces.length > EMBEDDED_TRACE_RETENTION_LIMIT) {
      this.traces.shift();
    }
  }

  traceList(
    requestedVersion: string,
    pageSize: number,
    cursor: string | null = null,
  ): EmbeddedTracePage | EmbeddedTraceApiError {
    const availability = this.traceApiAvailability(requestedVersion);
    if (availability !== null) {
      return availability;
    }
    const traces = this.newestTraces();
    const start = cursor === null ? 0 : this.cursorStart(cursor, traces);
    if (typeof start !== "number") {
      return start;
    }
    const boundedPageSize = Number.isFinite(pageSize)
      ? Math.max(1, Math.min(EMBEDDED_TRACE_MAX_PAGE_SIZE, Math.floor(pageSize)))
      : 1;
    const end = Math.min(start + boundedPageSize, traces.length);
    const lastSummary = end < traces.length ? traces[end - 1]?.summary : undefined;
    return {
      summaries: traces.slice(start, end).map((detail) => detail.summary),
      nextCursor: lastSummary === undefined ? null : this.cursorFor(lastSummary.traceId),
      retentionLimit: EMBEDDED_TRACE_RETENTION_LIMIT,
    };
  }

  traceGet(
    requestedVersion: string,
    traceId: string,
  ): EmbeddedTraceDetail | EmbeddedTraceApiError {
    const availability = this.traceApiAvailability(requestedVersion);
    if (availability !== null) {
      return availability;
    }
    return this.traces.find((detail) => detail.summary.traceId === traceId) ?? traceError("trace_not_found");
  }

  private traceApiAvailability(requestedVersion: string): EmbeddedTraceApiError | null {
    if (this.stopped) {
      return traceError("trace_api_unavailable");
    }
    if (requestedVersion !== EMBEDDED_TRACE_API_VERSION) {
      return traceError("incompatible_version");
    }
    return null;
  }

  private newestTraces(): EmbeddedTraceDetail[] {
    return [...this.traces].sort(
      (left, right) =>
        right.summary.completionSequence - left.summary.completionSequence ||
        left.summary.traceId.localeCompare(right.summary.traceId),
    );
  }

  private cursorFor(traceId: string): string {
    return `embedded-trace-cursor:${this.traceSession}:${traceId}`;
  }

  private cursorStart(
    cursor: string,
    traces: readonly EmbeddedTraceDetail[],
  ): number | EmbeddedTraceApiError {
    const separator = cursor.lastIndexOf(":");
    if (separator < 0 || cursor.slice(0, separator) !== `embedded-trace-cursor:${this.traceSession}`) {
      return traceError("invalid_cursor");
    }
    const position = traces.findIndex((detail) => detail.summary.traceId === cursor.slice(separator + 1));
    return position < 0 ? traceError("invalid_cursor") : position + 1;
  }

  private nextInstanceId(): string {
    this.nextInstance += 1;
    return paddedId("inst", this.nextInstance);
  }

  emit(
    eventType: EmbedderEvent["event_type"],
    sessionId: string | null,
    data: JsonValue,
  ): void {
    this.nextEvent += 1;
    const event: EmbedderEvent = {
      kind: "embedder_event",
      schema_version: EVENT_SCHEMA_VERSION,
      embedder_api_version: EMBEDDER_API_VERSION,
      event_id: paddedId("evt", this.nextEvent),
      sequence: this.nextEvent,
      event_type: eventType,
      workspace_id: this.workspaceId,
      app_id: this.appId,
      session_id: sessionId,
      data,
    };
    for (const subscriber of this.subscribers) {
      subscriber(event);
    }
    this.history.push(event);
  }

  subscribe(callback: EventCallback): void {
    for (const event of this.history) {
      callback(event);
    }
    this.subscribers.push(callback);
  }

  emitErrorEvent(
    sessionId: string | null,
    error: EmbedderError,
    data: { [key: string]: JsonValue },
  ): void {
    this.emit("error", sessionId, { ...data, error: errorValue(error) });
  }

  rejectedSubmit(targetId: string, error: EmbedderError): SubmitOutcome {
    this.emitErrorEvent(null, error, { target_id: targetId });
    return { sessionId: null, status: "rejected", error };
  }

  startCompatible(capabilityId: string, input: JsonValue): CompatibleStartOutcome {
    let error: EmbedderError | null = null;
    if (this.stopped) {
      error = runtimeStoppedError();
    } else {
      const platforms = this.compatibleTargets.get(capabilityId);
      if (platforms === undefined) {
        error = embedderError(
          "capability_not_compatible",
          `capability '${capabilityId}' is not a compatible-mode capability in this bundle`,
        );
      } else if (!platforms.includes(this.platform)) {
        error = embedderError(
          "platform_not_supported",
          `capability '${capabilityId}' permits platforms [${platforms.join(", ")}] ` +
            `but this embedder runs on '${this.platform}'`,
        );
      }
    }
    if (error !== null) {
      this.emitErrorEvent(null, error, { capability_id: capabilityId });
      return { instanceId: null, status: "error", error };
    }

    const instanceId = this.nextInstanceId();
    this.instances.set(instanceId, { capabilityId, state: "started" });
    this.emit("state_changed", null, {
      capability_id: capabilityId,
      instance_id: instanceId,
      state: "started",
      previous_state: null,
      input,
    });
    return { instanceId, status: "started", error: null };
  }

  transitionCompatible(
    capabilityId: string,
    instanceId: string | null,
    targetState: "stopped" | "killed",
  ): CompatibleLifecycleOutcome {
    if (this.stopped) {
      const error = runtimeStoppedError();
      this.emitErrorEvent(null, error, { capability_id: capabilityId });
      return { status: "error", error };
    }

    let selected: string[];
    if (instanceId !== null) {
      const instance = this.instances.get(instanceId);
      if (instance === undefined || instance.capabilityId !== capabilityId) {
        const error = embedderError(
          "instance_not_found",
          `no instance '${instanceId}' exists for capability '${capabilityId}'`,
        );
        this.emitErrorEvent(null, error, {
          capability_id: capabilityId,
          instance_id: instanceId,
        });
        return { status: "error", error };
      }
      if (instance.state !== "started") {
        const error = embedderError(
          "instance_not_running",
          `instance '${instanceId}' of capability '${capabilityId}' is not running`,
        );
        this.emitErrorEvent(null, error, {
          capability_id: capabilityId,
          instance_id: instanceId,
        });
        return { status: "error", error };
      }
      selected = [instanceId];
    } else {
      selected = [...this.instances.entries()]
        .filter(
          ([, instance]) =>
            instance.capabilityId === capabilityId && instance.state === "started",
        )
        .map(([id]) => id);
    }

    if (selected.length === 0) {
      const error = embedderError(
        "instance_not_running",
        `capability '${capabilityId}' has no running instances`,
      );
      this.emitErrorEvent(null, error, { capability_id: capabilityId });
      return { status: "error", error };
    }

    for (const id of selected) {
      this.setInstanceState(id, targetState);
    }
    return { status: targetState, error: null };
  }

  private setInstanceState(instanceId: string, targetState: InstanceState): void {
    const instance = this.instances.get(instanceId);
    if (instance === undefined) {
      return;
    }
    const previous = instance.state;
    instance.state = targetState;
    this.emit("state_changed", null, {
      capability_id: instance.capabilityId,
      instance_id: instanceId,
      state: targetState,
      previous_state: previous,
    });
  }

  shutdown(): { killedInstances: number } {
    if (this.stopped) {
      return { killedInstances: 0 };
    }
    const running = [...this.instances.entries()]
      .filter(([, instance]) => instance.state === "started")
      .map(([id]) => id);
    for (const id of running) {
      this.setInstanceState(id, "killed");
    }
    this.stopped = true;
    this.traces.length = 0;
    return { killedInstances: running.length };
  }

  evidence(runtimeImplementation: string, wasmComponents: JsonValue): JsonValue {
    return {
      kind: "embedder_release_evidence",
      schema_version: EVENT_SCHEMA_VERSION,
      package: { name: PACKAGE_NAME, version: PACKAGE_VERSION },
      embedder_api_version: EMBEDDER_API_VERSION,
      companion_apis: { "embedded-trace-api": EMBEDDED_TRACE_API_VERSION },
      conformance_version: EMBEDDER_CONFORMANCE_VERSION,
      runtime: { implementation: runtimeImplementation },
      supported_bundle_schema_versions: [...SUPPORTED_BUNDLE_SCHEMA_VERSIONS],
      bundle: {
        app_id: this.appId,
        app_version: this.appVersion,
        wasm_components: wasmComponents,
      },
      workspace_id: this.workspaceId,
      platform: this.platform,
    };
  }
}
