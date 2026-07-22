/**
 * Deterministic in-memory test double implementing the same public boundary
 * as the production embedder (spec 068 FR-006). It shares the event
 * envelope, identifier scheme, compatible lifecycle, and shutdown semantics;
 * only capability execution is replaced with scripted results. It contains
 * no business logic.
 */
import { EmbedderCore } from "./core.js";
import { EMBEDDED_TRACE_API_VERSION, embedderError, runtimeStoppedError } from "./types.js";
import type {
  CompatibleLifecycleOutcome,
  CompatibleStartOutcome,
  EmbeddedTraceApi,
  EmbeddedTraceApiError,
  EmbeddedTraceDetail,
  EmbeddedTracePage,
  EventCallback,
  JsonValue,
  ShutdownOutcome,
  SubmitOutcome,
  TraverseEmbedderApi,
} from "./types.js";

/** Configuration for the deterministic test double. */
export interface EmbedderTestDoubleConfig {
  readonly workspaceId?: string;
  readonly appId?: string;
  readonly appVersion?: string;
  readonly platform?: string;
}

type ScriptedResult =
  | { readonly kind: "output"; readonly output: JsonValue }
  | { readonly kind: "error"; readonly code: string; readonly message: string };

export class EmbedderTestDouble implements TraverseEmbedderApi, EmbeddedTraceApi {
  private readonly core: EmbedderCore;
  private readonly scripted = new Map<string, ScriptedResult>();

  constructor(config: EmbedderTestDoubleConfig = {}) {
    this.core = new EmbedderCore(
      config.workspaceId ?? "local-default",
      config.appId ?? "test-app",
      config.appVersion ?? "1.0.0",
      config.platform ?? "web",
      new Map(),
    );
  }

  /** Scripts `submit(targetId, _)` to succeed with `output`. */
  withTargetOutput(targetId: string, output: JsonValue): this {
    this.scripted.set(targetId, { kind: "output", output });
    return this;
  }

  /** Scripts `submit(targetId, _)` to fail with a runtime-shaped error. */
  withTargetError(targetId: string, code: string, message: string): this {
    this.scripted.set(targetId, { kind: "error", code, message });
    return this;
  }

  /** Declares a compatible-mode capability with a platform allowlist. */
  withCompatibleTarget(capabilityId: string, platforms: readonly string[]): this {
    this.core.compatibleTargets.set(capabilityId, platforms);
    return this;
  }

  submit(targetId: string, input: JsonValue): SubmitOutcome {
    void input;
    if (this.core.stopped) {
      return this.core.rejectedSubmit(targetId, runtimeStoppedError());
    }
    const result = this.scripted.get(targetId);
    if (result === undefined) {
      return this.core.rejectedSubmit(
        targetId,
        embedderError(
          "target_not_found",
          `'${targetId}' is neither a bundled workflow nor a bundled capability`,
        ),
      );
    }

    const sessionId = this.core.nextSessionId();
    const requestId = this.core.nextRequestId();
    const executionId = `exec_${requestId}`;
    this.core.recordTrace({
      executionId,
      targetId,
      outcome: result.kind === "output" ? "completed" : "error",
      phases: [{ code: result.kind === "output" ? "completed" : "error" }],
      selectedTarget: { targetId, targetVersion: "1.0.0" },
      placement: null,
      failureCode: result.kind === "error" ? result.code : null,
      stateMachineValid: true,
    });
    this.core.emit("capability_invoked", sessionId, {
      execution_id: executionId,
      capability_id: targetId,
      capability_version: "1.0.0",
    });
    if (result.kind === "output") {
      this.core.emit("capability_result", sessionId, {
        execution_id: executionId,
        capability_id: targetId,
        status: "completed",
        output: result.output,
      });
    } else {
      this.core.emit("error", sessionId, {
        execution_id: executionId,
        capability_id: targetId,
        status: "error",
        error: { code: result.code, message: result.message, details: {} },
      });
    }
    return { sessionId, status: "accepted", error: null };
  }

  subscribe(callback: EventCallback): void {
    this.core.subscribe(callback);
  }

  embeddedTraceApiVersion(): string {
    return EMBEDDED_TRACE_API_VERSION;
  }

  traceList(
    requestedVersion: string,
    pageSize: number,
    cursor: string | null = null,
  ): EmbeddedTracePage | EmbeddedTraceApiError {
    return this.core.traceList(requestedVersion, pageSize, cursor);
  }

  traceGet(
    requestedVersion: string,
    traceId: string,
  ): EmbeddedTraceDetail | EmbeddedTraceApiError {
    return this.core.traceGet(requestedVersion, traceId);
  }

  startCompatible(capabilityId: string, input: JsonValue): CompatibleStartOutcome {
    return this.core.startCompatible(capabilityId, input);
  }

  stopCompatible(
    capabilityId: string,
    instanceId: string | null = null,
  ): CompatibleLifecycleOutcome {
    return this.core.transitionCompatible(capabilityId, instanceId, "stopped");
  }

  killCompatible(
    capabilityId: string,
    instanceId: string | null = null,
  ): CompatibleLifecycleOutcome {
    return this.core.transitionCompatible(capabilityId, instanceId, "killed");
  }

  shutdown(): ShutdownOutcome {
    return this.core.shutdown();
  }

  releaseEvidence(): JsonValue {
    return this.core.evidence("test-double", []);
  }
}
