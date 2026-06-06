//! Integration tests for the six MCP tools exposed by traverse-mcp.
//!
//! Governed by spec 015-capability-discovery-mcp

use std::sync::Arc;
use traverse_contracts::{
    BinaryFormat as ContractBinaryFormat, Condition, DependencyArtifactType, DependencyReference,
    Entrypoint, EntrypointKind, EventReference, Execution, ExecutionConstraints, ExecutionTarget,
    FilesystemAccess, HostApiAccess, IdReference, Lifecycle, NetworkAccess, Owner, Provenance,
    ProvenanceSource, SchemaContainer, ServiceType, SideEffect, SideEffectKind,
};
use traverse_mcp::{
    McpContext,
    tools::{
        capabilities::{CapabilityFilter, get_capability, list_capabilities},
        events::{get_event_type, list_event_types},
        traces::{GetTraceRequest, ListTracesRequest, get_trace, list_traces},
    },
};
use traverse_registry::{
    ArtifactDigests, BinaryFormat, BinaryReference, CapabilityArtifactRecord,
    CapabilityRegistration, CapabilityRegistry, ComposabilityMetadata, CompositionKind,
    CompositionPattern, ImplementationKind, RegistryProvenance, RegistryScope, SourceKind,
    SourceReference,
};
use traverse_runtime::{
    events::{
        catalog::{EventCatalog, EventCatalogEntry},
        types::LifecycleStatus,
    },
    trace::{PrivateTraceEntry, PublicTraceEntry, TraceOutcome, TraceStore},
};

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

fn capability_contract() -> traverse_contracts::CapabilityContract {
    traverse_contracts::CapabilityContract {
        kind: "capability_contract".to_string(),
        schema_version: "1.0.0".to_string(),
        id: "content.comments.create-comment-draft".to_string(),
        namespace: "content.comments".to_string(),
        name: "create-comment-draft".to_string(),
        version: "1.0.0".to_string(),
        lifecycle: Lifecycle::Active,
        owner: Owner {
            team: "comments".to_string(),
            contact: "comments@example.com".to_string(),
        },
        summary: "Create a comment draft.".to_string(),
        description: "Create a deterministic comment draft.".to_string(),
        inputs: SchemaContainer {
            schema: serde_json::json!({
                "type": "object",
                "required": ["comment_text", "resource_id"],
                "properties": {
                    "comment_text": {"type": "string"},
                    "resource_id": {"type": "string"}
                }
            }),
        },
        outputs: SchemaContainer {
            schema: serde_json::json!({
                "type": "object",
                "required": ["draft_id"],
                "properties": {
                    "draft_id": {"type": "string"}
                }
            }),
        },
        preconditions: vec![Condition {
            id: "authenticated".to_string(),
            description: "Caller is authenticated.".to_string(),
        }],
        postconditions: vec![Condition {
            id: "draft_created".to_string(),
            description: "Draft id is produced.".to_string(),
        }],
        side_effects: vec![SideEffect {
            kind: SideEffectKind::MemoryOnly,
            description: "Creates draft state.".to_string(),
        }],
        emits: vec![EventReference {
            event_id: "content.comments.draft-created".to_string(),
            version: "1.0.0".to_string(),
        }],
        consumes: Vec::new(),
        permissions: vec![IdReference {
            id: "comments.create".to_string(),
        }],
        execution: Execution {
            binary_format: ContractBinaryFormat::Wasm,
            entrypoint: Entrypoint {
                kind: EntrypointKind::WasiCommand,
                command: "run".to_string(),
            },
            preferred_targets: vec![ExecutionTarget::Local],
            constraints: ExecutionConstraints {
                host_api_access: HostApiAccess::None,
                network_access: NetworkAccess::Forbidden,
                filesystem_access: FilesystemAccess::None,
            },
        },
        policies: vec![IdReference {
            id: "policy.comments.default".to_string(),
        }],
        dependencies: vec![DependencyReference {
            artifact_type: DependencyArtifactType::Event,
            id: "content.comments.draft-created".to_string(),
            version: "1.0.0".to_string(),
        }],
        provenance: Provenance {
            source: ProvenanceSource::Greenfield,
            author: "test".to_string(),
            created_at: "2026-04-09T00:00:00Z".to_string(),
            spec_ref: None,
            adr_refs: Vec::new(),
            exception_refs: Vec::new(),
        },
        evidence: Vec::new(),
        service_type: ServiceType::Stateless,
        permitted_targets: vec![
            ExecutionTarget::Local,
            ExecutionTarget::Cloud,
            ExecutionTarget::Edge,
        ],
        event_trigger: None,
        connector_requirements: Vec::new(),
        state_schema: None,
    }
}

fn subscribable_capability_contract() -> traverse_contracts::CapabilityContract {
    let mut contract = capability_contract();
    contract.id = "content.comments.notify-subscriber".to_string();
    contract.name = "notify-subscriber".to_string();
    contract.service_type = ServiceType::Subscribable;
    contract.event_trigger = Some("content.comments.draft-created".to_string());
    contract.emits = Vec::new();
    contract.consumes = vec![EventReference {
        event_id: "content.comments.draft-created".to_string(),
        version: "1.0.0".to_string(),
    }];
    // Only local — no Cloud target, to distinguish from the stateless capability
    contract.permitted_targets = vec![ExecutionTarget::Local];
    contract
}

fn capability_artifact_record(id: &str) -> CapabilityArtifactRecord {
    CapabilityArtifactRecord {
        artifact_ref: format!("artifact:{id}:1.0.0"),
        implementation_kind: ImplementationKind::Executable,
        source: SourceReference {
            kind: SourceKind::Local,
            location: "src/".to_string(),
        },
        binary: Some(BinaryReference {
            format: BinaryFormat::Wasm,
            location: format!("artifacts/{id}.wasm"),
        }),
        workflow_ref: None,
        digests: ArtifactDigests {
            source_digest: "src-hash".to_string(),
            binary_digest: Some("bin-hash".to_string()),
        },
        provenance: RegistryProvenance {
            source: "test".to_string(),
            author: "test".to_string(),
            created_at: "2026-04-09T00:00:00Z".to_string(),
        },
    }
}

fn composability() -> ComposabilityMetadata {
    ComposabilityMetadata {
        kind: CompositionKind::Atomic,
        patterns: vec![CompositionPattern::Sequential],
        provides: vec!["draft".to_string()],
        requires: Vec::new(),
    }
}

fn capability_registry_with_two_capabilities() -> Result<CapabilityRegistry, String> {
    let mut registry = CapabilityRegistry::new();

    let stateless_contract = capability_contract();
    let stateless_id = stateless_contract.id.clone();
    registry
        .register(CapabilityRegistration {
            scope: RegistryScope::Public,
            contract: stateless_contract,
            contract_path: format!("registry/public/{stateless_id}/1.0.0/contract.json"),
            artifact: capability_artifact_record(&stateless_id),
            registered_at: "2026-04-09T00:00:00Z".to_string(),
            tags: vec!["comments".to_string()],
            composability: composability(),
            governing_spec: "005-capability-registry".to_string(),
            validator_version: "v1".to_string(),
        })
        .map_err(|e| format!("{e:?}"))?;

    let subscribable_contract = subscribable_capability_contract();
    let subscribable_id = subscribable_contract.id.clone();
    registry
        .register(CapabilityRegistration {
            scope: RegistryScope::Public,
            contract: subscribable_contract,
            contract_path: format!("registry/public/{subscribable_id}/1.0.0/contract.json"),
            artifact: capability_artifact_record(&subscribable_id),
            registered_at: "2026-04-09T00:00:00Z".to_string(),
            tags: vec!["comments".to_string()],
            composability: composability(),
            governing_spec: "005-capability-registry".to_string(),
            validator_version: "v1".to_string(),
        })
        .map_err(|e| format!("{e:?}"))?;

    Ok(registry)
}

fn event_catalog_with_one_entry() -> Result<EventCatalog, String> {
    let catalog = EventCatalog::new();
    catalog
        .register(EventCatalogEntry {
            event_type: "content.comments.draft-created".to_string(),
            owner: "content.comments.create-comment-draft".to_string(),
            version: "1.0.0".to_string(),
            lifecycle_status: LifecycleStatus::Active,
            consumer_count: 0,
        })
        .map_err(|e| format!("{e:?}"))?;
    Ok(catalog)
}

fn trace_store_with_one_entry() -> (TraceStore, String) {
    let mut store = TraceStore::new();
    let trace_id = "trace-test-001".to_string();
    let public = PublicTraceEntry::new(
        trace_id.clone(),
        "content.comments.create-comment-draft".to_string(),
        "local".to_string(),
        TraceOutcome::Success,
        42,
        "2026-04-09T00:00:00Z".to_string(),
    );
    let private = PrivateTraceEntry::new(
        trace_id.clone(),
        r#"{"comment_text":"hi","resource_id":"r1"}"#,
        r#"{"draft_id":"d1"}"#,
        40,
    );
    store.insert(public, Some(private));
    (store, trace_id)
}

fn mcp_context(
    registry: CapabilityRegistry,
    catalog: EventCatalog,
    store: TraceStore,
) -> McpContext {
    McpContext::new(Arc::new(registry), Arc::new(catalog), Arc::new(store))
}

// ---------------------------------------------------------------------------
// FR-001: list_capabilities
// ---------------------------------------------------------------------------

#[test]
fn list_capabilities_returns_all_when_no_filter() -> Result<(), String> {
    let registry = capability_registry_with_two_capabilities()?;
    let summaries = list_capabilities(&registry, None);
    assert_eq!(
        summaries.len(),
        2,
        "expected 2 capabilities, got {}",
        summaries.len()
    );
    Ok(())
}

#[test]
fn list_capabilities_filters_by_service_type() -> Result<(), String> {
    let registry = capability_registry_with_two_capabilities()?;
    let filter = CapabilityFilter {
        service_type: Some(ServiceType::Subscribable),
        permitted_targets: Vec::new(),
    };
    let summaries = list_capabilities(&registry, Some(&filter));
    assert_eq!(summaries.len(), 1, "expected 1 subscribable capability");
    assert_eq!(summaries[0].id, "content.comments.notify-subscriber");
    assert_eq!(summaries[0].service_type, ServiceType::Subscribable);
    Ok(())
}

#[test]
fn list_capabilities_filters_by_permitted_targets() -> Result<(), String> {
    let registry = capability_registry_with_two_capabilities()?;
    let filter = CapabilityFilter {
        service_type: None,
        permitted_targets: vec![ExecutionTarget::Cloud],
    };
    let summaries = list_capabilities(&registry, Some(&filter));
    // Only the stateless capability has Cloud in permitted_targets
    assert_eq!(
        summaries.len(),
        1,
        "expected 1 capability with Cloud target"
    );
    assert_eq!(summaries[0].id, "content.comments.create-comment-draft");
    Ok(())
}

#[test]
fn list_capabilities_returns_expected_fields() -> Result<(), String> {
    let registry = capability_registry_with_two_capabilities()?;
    let filter = CapabilityFilter {
        service_type: Some(ServiceType::Stateless),
        permitted_targets: Vec::new(),
    };
    let summaries = list_capabilities(&registry, Some(&filter));
    assert_eq!(summaries.len(), 1);
    let s = &summaries[0];
    assert_eq!(s.id, "content.comments.create-comment-draft");
    assert_eq!(s.name, "create-comment-draft");
    assert!(!s.description.is_empty());
    assert!(!s.permitted_targets.is_empty());
    Ok(())
}

// ---------------------------------------------------------------------------
// FR-002: get_capability
// ---------------------------------------------------------------------------

#[test]
fn get_capability_returns_full_contract_json() -> Result<(), String> {
    let registry = capability_registry_with_two_capabilities()?;
    let result = get_capability(&registry, "content.comments.create-comment-draft")
        .map_err(|e| format!("unexpected error: {e:?}"))?;
    assert_eq!(
        result["id"].as_str(),
        Some("content.comments.create-comment-draft")
    );
    Ok(())
}

#[test]
fn get_capability_returns_not_found_for_missing_id() -> Result<(), String> {
    let registry = capability_registry_with_two_capabilities()?;
    let err = match get_capability(&registry, "does.not.exist") {
        Ok(v) => return Err(format!("expected error but got Ok({v:?})")),
        Err(err) => err,
    };
    assert!(
        format!("{err:?}").contains("NotFound"),
        "expected NotFound, got {err:?}"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// FR-003: list_event_types
// ---------------------------------------------------------------------------

#[test]
fn list_event_types_returns_all_catalog_entries() -> Result<(), String> {
    let catalog = event_catalog_with_one_entry()?;
    let entries = list_event_types(&catalog);
    assert_eq!(entries.len(), 1, "expected 1 event type");
    assert_eq!(entries[0].event_type, "content.comments.draft-created");
    assert_eq!(entries[0].owner, "content.comments.create-comment-draft");
    Ok(())
}

// ---------------------------------------------------------------------------
// FR-004: get_event_type
// ---------------------------------------------------------------------------

#[test]
fn get_event_type_returns_entry_for_known_type() -> Result<(), String> {
    let catalog = event_catalog_with_one_entry()?;
    let entry = get_event_type(&catalog, "content.comments.draft-created")
        .ok_or("expected Some but got None")?;
    assert_eq!(entry.event_type, "content.comments.draft-created");
    assert_eq!(entry.version, "1.0.0");
    Ok(())
}

#[test]
fn get_event_type_returns_none_for_unknown_type() -> Result<(), String> {
    let catalog = event_catalog_with_one_entry()?;
    let entry = get_event_type(&catalog, "does.not.exist");
    assert!(entry.is_none(), "expected None for unknown event type");
    Ok(())
}

// ---------------------------------------------------------------------------
// FR-005: list_traces
// ---------------------------------------------------------------------------

#[test]
fn list_traces_returns_all_public_entries_when_no_filter() {
    let (store, _) = trace_store_with_one_entry();
    let entries = list_traces(
        &store,
        &ListTracesRequest {
            capability_id: None,
        },
    );
    assert_eq!(entries.len(), 1, "expected 1 trace entry");
    assert_eq!(
        entries[0].capability_id,
        "content.comments.create-comment-draft"
    );
}

#[test]
fn list_traces_filters_by_capability_id() {
    let (store, _) = trace_store_with_one_entry();

    let matching = list_traces(
        &store,
        &ListTracesRequest {
            capability_id: Some("content.comments.create-comment-draft".to_string()),
        },
    );
    assert_eq!(matching.len(), 1, "expected 1 matching trace");

    let none = list_traces(
        &store,
        &ListTracesRequest {
            capability_id: Some("other.capability".to_string()),
        },
    );
    assert_eq!(none.len(), 0, "expected 0 traces for non-matching filter");
}

// ---------------------------------------------------------------------------
// FR-006: get_trace
// ---------------------------------------------------------------------------

#[test]
fn get_trace_returns_public_entry_always() -> Result<(), String> {
    let (store, trace_id) = trace_store_with_one_entry();
    let response = get_trace(
        &store,
        &GetTraceRequest {
            trace_id: trace_id.clone(),
            include_private: false,
        },
    )
    .ok_or("expected Some but got None")?;
    assert_eq!(response.public.id, trace_id);
    assert!(
        response.private.is_none(),
        "private should be None when include_private is false"
    );
    Ok(())
}

#[test]
fn get_trace_includes_private_when_flag_is_true() -> Result<(), String> {
    let (store, trace_id) = trace_store_with_one_entry();
    let response = get_trace(
        &store,
        &GetTraceRequest {
            trace_id: trace_id.clone(),
            include_private: true,
        },
    )
    .ok_or("expected Some but got None")?;
    assert_eq!(response.public.id, trace_id);
    let private = response
        .private
        .ok_or("expected private entry but got None")?;
    assert_eq!(private.trace_id, trace_id);
    assert!(!private.inputs_hash.is_empty());
    assert!(!private.outputs_hash.is_empty());
    Ok(())
}

#[test]
fn get_trace_returns_none_for_unknown_trace_id() {
    let (store, _) = trace_store_with_one_entry();
    let response = get_trace(
        &store,
        &GetTraceRequest {
            trace_id: "does-not-exist".to_string(),
            include_private: true,
        },
    );
    assert!(response.is_none(), "expected None for unknown trace id");
}

// ---------------------------------------------------------------------------
// FR-007: McpContext
// ---------------------------------------------------------------------------

#[test]
fn mcp_context_holds_injected_registries() -> Result<(), String> {
    let registry = capability_registry_with_two_capabilities()?;
    let catalog = event_catalog_with_one_entry()?;
    let (store, _) = trace_store_with_one_entry();
    let ctx = mcp_context(registry, catalog, store);

    // Verify all three are accessible through McpContext
    let caps = list_capabilities(&ctx.capability_registry, None);
    assert_eq!(caps.len(), 2);

    let events = list_event_types(&ctx.event_catalog);
    assert_eq!(events.len(), 1);

    let traces = list_traces(
        &ctx.trace_store,
        &ListTracesRequest {
            capability_id: None,
        },
    );
    assert_eq!(traces.len(), 1);
    Ok(())
}

// ---------------------------------------------------------------------------
// FR-008: JSON serialization
// ---------------------------------------------------------------------------

#[test]
fn list_capabilities_serializes_to_valid_json() -> Result<(), String> {
    let registry = capability_registry_with_two_capabilities()?;
    let summaries = list_capabilities(&registry, None);
    let json =
        serde_json::to_string(&summaries).map_err(|e| format!("serialization failed: {e}"))?;
    assert!(json.starts_with('['), "expected JSON array");
    let parsed: serde_json::Value =
        serde_json::from_str(&json).map_err(|e| format!("parse failed: {e}"))?;
    assert!(parsed.is_array());
    Ok(())
}

#[test]
fn get_capability_produces_valid_json_contract() -> Result<(), String> {
    let registry = capability_registry_with_two_capabilities()?;
    let contract_json = get_capability(&registry, "content.comments.create-comment-draft")
        .map_err(|e| format!("unexpected error: {e:?}"))?;
    assert!(contract_json.is_object(), "expected JSON object");
    assert_eq!(
        contract_json["id"].as_str(),
        Some("content.comments.create-comment-draft")
    );
    assert_eq!(contract_json["service_type"].as_str(), Some("stateless"));
    Ok(())
}
