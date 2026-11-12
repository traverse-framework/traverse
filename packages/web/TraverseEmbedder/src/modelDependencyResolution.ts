/**
 * Spec 045 model-dependency resolution for the browser embedder
 * (Decision 95 / `#1835`). Ports `traverse_registry::resolve_model_dependency`
 * with a browser availability probe that never HTTP-probes Ollama and only
 * admits activated Spec 138 `exact-ref.wasm-cpu` pins.
 */

export const OLLAMA_LOCAL_GENERATE = "ollama.local.generate" as const;
export const EXACT_REF_WASM_CPU = "exact-ref.wasm-cpu" as const;

/** Wire values of `traverse_contracts::ExecutionTarget`. */
export type ExecutionTarget = "local" | "browser" | "cloud" | "edge";

export type ModelResolutionPhase = "setup" | "execution";

export type ModelCandidateRejectionCode =
  | "model_provider_unavailable"
  | "model_candidate_unavailable"
  | "model_interface_unsupported"
  | "model_context_window_insufficient"
  | "model_candidate_config_invalid"
  | "model_dependency_unsatisfied";

export type ModelCandidateReadiness = "ready" | "rejected";

/** Spec 045 FR-012a / Decision 95 trust class on generate/resolution evidence. */
export type InferenceTrustClass = "verified_signed_package" | "local_unverified_daemon";

export type ModelSelectionPolicy = {
  readonly strategy: string;
  readonly allow_fallback: boolean;
};

export type ModelCandidate = {
  readonly candidate_id: string;
  readonly provider_capability_id: string;
  readonly provider_implementation_id: string;
  readonly model_identifier: string;
  readonly placement_target: ExecutionTarget;
  readonly priority: number;
  readonly required_provider_config_keys?: readonly string[];
  readonly metadata?: Record<string, unknown>;
};

export type ApplicationModelDependency = {
  readonly interface_id: string;
  readonly version_range: string;
  readonly selection_policy: ModelSelectionPolicy;
  readonly required_capabilities: readonly string[];
  readonly minimum_context_window: number;
  readonly candidates: readonly ModelCandidate[];
};

export type ModelResolutionRequest = {
  readonly phase: ModelResolutionPhase;
  readonly requested_interface_id: string;
  readonly requested_placement: ExecutionTarget;
};

export type ModelCandidateAvailability = {
  readonly provider_available: boolean;
  readonly model_available: boolean;
  readonly failure_code?: ModelCandidateRejectionCode;
  readonly reason?: string;
};

export type ModelAvailabilityProbe = {
  checkCandidate(
    dependency: ApplicationModelDependency,
    candidate: ModelCandidate,
  ): ModelCandidateAvailability;
};

export type ModelCandidateEvaluation = {
  readonly candidate_id: string;
  readonly provider_capability_id: string;
  readonly provider_implementation_id: string;
  readonly model_identifier: string;
  readonly placement_target: ExecutionTarget;
  readonly priority: number;
  readonly readiness: ModelCandidateReadiness;
  readonly rejection_code?: ModelCandidateRejectionCode;
  readonly reason: string;
  readonly manifest_order: number;
};

export type SelectedModelCandidate = {
  readonly candidate_id: string;
  readonly provider_capability_id: string;
  readonly provider_implementation_id: string;
  readonly model_identifier: string;
  readonly placement_target: ExecutionTarget;
  readonly priority: number;
  readonly selection_reason: string;
};

export type ModelResolutionEvidence = {
  readonly phase: ModelResolutionPhase;
  readonly interface_id: string;
  readonly requested_interface_id: string;
  readonly requested_placement: ExecutionTarget;
  readonly selected?: SelectedModelCandidate;
  readonly candidates: readonly ModelCandidateEvaluation[];
  readonly failure_code?: ModelCandidateRejectionCode;
};

/** Browser-facing result: Spec 045 evidence plus FR-012a `trust_class` when selected. */
export type BrowserModelResolutionResult = {
  readonly resolution: ModelResolutionEvidence;
  readonly trust_class?: InferenceTrustClass;
};

export function availabilityReady(): ModelCandidateAvailability {
  return { provider_available: true, model_available: true };
}

export function availabilityRejected(
  code: ModelCandidateRejectionCode,
  reason: string,
): ModelCandidateAvailability {
  return {
    provider_available: code !== "model_provider_unavailable",
    model_available: code !== "model_candidate_unavailable",
    failure_code: code,
    reason,
  };
}

/**
 * Browser probe (Decision 95 / `#1835`): never HTTP-probes Ollama; every
 * `ollama.local.generate` candidate is unavailable. An `exact-ref.wasm-cpu`
 * candidate is ready only when its referenced Spec 044 pin is activated.
 */
export class BrowserModelAvailabilityProbe implements ModelAvailabilityProbe {
  private readonly activatedExactRefPins: ReadonlySet<string>;

  constructor(activatedExactRefPins: Iterable<readonly [string, string]> = []) {
    this.activatedExactRefPins = new Set(
      [...activatedExactRefPins].map(([modelId, version]) => pinKey(modelId, version)),
    );
  }

  checkCandidate(
    _dependency: ApplicationModelDependency,
    candidate: ModelCandidate,
  ): ModelCandidateAvailability {
    if (candidate.provider_implementation_id === OLLAMA_LOCAL_GENERATE) {
      return availabilityRejected(
        "model_provider_unavailable",
        "ollama.local.generate is unavailable in the browser",
      );
    }
    if (candidate.provider_implementation_id === EXACT_REF_WASM_CPU) {
      return checkExactRefCandidateAvailability(candidate, this.activatedExactRefPins);
    }
    return availabilityRejected(
      "model_candidate_config_invalid",
      `unsupported provider_implementation_id: ${candidate.provider_implementation_id}`,
    );
  }
}

function pinKey(modelId: string, version: string): string {
  return `${modelId}\u0000${version}`;
}

function checkExactRefCandidateAvailability(
  candidate: ModelCandidate,
  activatedExactRefPins: ReadonlySet<string>,
): ModelCandidateAvailability {
  const modelId = metadataString(candidate.metadata, "exact_ref_model_id");
  if (modelId === undefined) {
    return availabilityRejected(
      "model_candidate_config_invalid",
      "exact-ref candidate is missing exact_ref_model_id metadata",
    );
  }
  const version = metadataString(candidate.metadata, "exact_ref_version");
  if (version === undefined) {
    return availabilityRejected(
      "model_candidate_config_invalid",
      "exact-ref candidate is missing exact_ref_version metadata",
    );
  }
  if (activatedExactRefPins.has(pinKey(modelId, version))) {
    return availabilityReady();
  }
  return availabilityRejected(
    "model_provider_unavailable",
    "referenced exact_model_dependencies pin has no activated traverse.model-runtime binding",
  );
}

function metadataString(
  metadata: Record<string, unknown> | undefined,
  key: string,
): string | undefined {
  const value = metadata?.[key];
  return typeof value === "string" ? value : undefined;
}

/**
 * Spec 045 resolver: same heuristic order as Rust
 * `traverse_registry::resolve_model_dependency`.
 */
export function resolveModelDependency(
  dependency: ApplicationModelDependency,
  request: ModelResolutionRequest,
  probe: ModelAvailabilityProbe,
): ModelResolutionEvidence {
  const evaluations: ModelCandidateEvaluation[] = [];
  const passing: ModelCandidateEvaluation[] = [];

  dependency.candidates.forEach((candidate, manifestOrder) => {
    const evaluation = evaluateCandidate(dependency, request, candidate, manifestOrder, probe);
    if (evaluation.readiness === "ready") {
      passing.push(evaluation);
    }
    evaluations.push(evaluation);
  });

  passing.sort((left, right) => {
    if (right.priority !== left.priority) {
      return right.priority - left.priority;
    }
    return left.manifest_order - right.manifest_order;
  });

  const first = passing[0];
  const second = passing[1];
  const selected: SelectedModelCandidate | undefined = first
    ? {
        candidate_id: first.candidate_id,
        provider_capability_id: first.provider_capability_id,
        provider_implementation_id: first.provider_implementation_id,
        model_identifier: first.model_identifier,
        placement_target: first.placement_target,
        priority: first.priority,
        selection_reason:
          second !== undefined && second.priority === first.priority
            ? "selected by priority tie using manifest order"
            : "selected highest-priority passing candidate",
      }
    : undefined;

  return {
    phase: request.phase,
    interface_id: dependency.interface_id,
    requested_interface_id: request.requested_interface_id,
    requested_placement: request.requested_placement,
    ...(selected ? { selected } : {}),
    candidates: evaluations,
    ...(selected ? {} : { failure_code: "model_dependency_unsatisfied" as const }),
  };
}

/**
 * Resolve under the browser probe and attach FR-012a `trust_class` for the
 * selected candidate (Decision 95 / `#1835`).
 */
export function resolveBrowserModelDependency(
  dependency: ApplicationModelDependency,
  request: ModelResolutionRequest,
  activatedExactRefPins: Iterable<readonly [string, string]> = [],
): BrowserModelResolutionResult {
  const resolution = resolveModelDependency(
    dependency,
    request,
    new BrowserModelAvailabilityProbe(activatedExactRefPins),
  );
  const trustClass = trustClassForProvider(resolution.selected?.provider_implementation_id);
  return trustClass === undefined
    ? { resolution }
    : { resolution, trust_class: trustClass };
}

export function trustClassForProvider(
  providerImplementationId: string | undefined,
): InferenceTrustClass | undefined {
  if (providerImplementationId === EXACT_REF_WASM_CPU) {
    return "verified_signed_package";
  }
  if (providerImplementationId === OLLAMA_LOCAL_GENERATE) {
    return "local_unverified_daemon";
  }
  return undefined;
}

/**
 * Map a selected `exact-ref.wasm-cpu` candidate to the Spec 044 pin identity
 * ExactModelBrowserHost needs before `model.execute`.
 */
export function exactRefPinFromSelected(selected: SelectedModelCandidate, candidate: ModelCandidate): {
  readonly model_id: string;
  readonly version: string;
} {
  if (selected.provider_implementation_id !== EXACT_REF_WASM_CPU) {
    throw new Error("selected candidate is not exact-ref.wasm-cpu");
  }
  const modelId = metadataString(candidate.metadata, "exact_ref_model_id");
  const version = metadataString(candidate.metadata, "exact_ref_version");
  if (modelId === undefined || version === undefined) {
    throw new Error("exact-ref candidate is missing exact_ref_model_id/exact_ref_version metadata");
  }
  return { model_id: modelId, version };
}

function evaluateCandidate(
  dependency: ApplicationModelDependency,
  request: ModelResolutionRequest,
  candidate: ModelCandidate,
  manifestOrder: number,
  probe: ModelAvailabilityProbe,
): ModelCandidateEvaluation {
  const availability = probe.checkCandidate(dependency, candidate);
  if (availability.failure_code !== undefined) {
    return rejectedCandidate(
      candidate,
      manifestOrder,
      availability.failure_code,
      availability.reason ?? availability.failure_code,
    );
  }
  if (!availability.provider_available) {
    return rejectedCandidate(
      candidate,
      manifestOrder,
      "model_provider_unavailable",
      "provider unavailable",
    );
  }
  if (!availability.model_available) {
    return rejectedCandidate(
      candidate,
      manifestOrder,
      "model_candidate_unavailable",
      "model unavailable",
    );
  }
  if (
    candidate.provider_capability_id !== request.requested_interface_id ||
    candidate.provider_capability_id !== dependency.interface_id ||
    !supportsRequiredCapabilities(candidate.metadata, dependency.required_capabilities)
  ) {
    return rejectedCandidate(
      candidate,
      manifestOrder,
      "model_interface_unsupported",
      "candidate does not support requested inference interface",
    );
  }
  if (candidate.placement_target !== request.requested_placement) {
    return rejectedCandidate(
      candidate,
      manifestOrder,
      "model_candidate_config_invalid",
      "candidate placement does not match requested placement policy",
    );
  }
  const contextWindow = metadataNumber(candidate.metadata, "model_context_window") ?? 0;
  if (contextWindow < dependency.minimum_context_window) {
    return rejectedCandidate(
      candidate,
      manifestOrder,
      "model_context_window_insufficient",
      `model context window ${contextWindow} is below required ${dependency.minimum_context_window}`,
    );
  }
  return {
    candidate_id: candidate.candidate_id,
    provider_capability_id: candidate.provider_capability_id,
    provider_implementation_id: candidate.provider_implementation_id,
    model_identifier: candidate.model_identifier,
    placement_target: candidate.placement_target,
    priority: candidate.priority,
    readiness: "ready",
    reason: "candidate passed availability, interface, placement, and context checks",
    manifest_order: manifestOrder,
  };
}

function rejectedCandidate(
  candidate: ModelCandidate,
  manifestOrder: number,
  code: ModelCandidateRejectionCode,
  reason: string,
): ModelCandidateEvaluation {
  return {
    candidate_id: candidate.candidate_id,
    provider_capability_id: candidate.provider_capability_id,
    provider_implementation_id: candidate.provider_implementation_id,
    model_identifier: candidate.model_identifier,
    placement_target: candidate.placement_target,
    priority: candidate.priority,
    readiness: "rejected",
    rejection_code: code,
    reason,
    manifest_order: manifestOrder,
  };
}

function supportsRequiredCapabilities(
  metadata: Record<string, unknown> | undefined,
  required: readonly string[],
): boolean {
  if (required.length === 0) {
    return true;
  }
  const capabilities = metadata?.capabilities;
  if (!Array.isArray(capabilities)) {
    return false;
  }
  return required.every((needed) =>
    capabilities.some((capability) => typeof capability === "string" && capability === needed),
  );
}

function metadataNumber(
  metadata: Record<string, unknown> | undefined,
  key: string,
): number | undefined {
  const value = metadata?.[key];
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}
