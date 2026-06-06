use crate::{
    CapabilityArtifactRecord, CapabilityRegistry, ImplementationKind, LookupScope, RegistryScope,
    WorkflowReference,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use traverse_contracts::{ErrorSeverity, EventReference, Lifecycle, Owner, SchemaContainer};

const WORKFLOW_KIND: &str = "workflow_definition";
const WORKFLOW_SCHEMA_VERSION: &str = "1.0.0";
const WORKFLOW_GOVERNING_SPEC: &str = "007-workflow-registry-traversal";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub kind: String,
    pub schema_version: String,
    pub id: String,
    pub name: String,
    pub version: String,
    pub lifecycle: Lifecycle,
    pub owner: Owner,
    pub summary: String,
    pub inputs: SchemaContainer,
    pub outputs: SchemaContainer,
    pub nodes: Vec<WorkflowNode>,
    pub edges: Vec<WorkflowEdge>,
    pub start_node: String,
    pub terminal_nodes: Vec<String>,
    pub tags: Vec<String>,
    pub governing_spec: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowNode {
    pub node_id: String,
    pub capability_id: String,
    pub capability_version: String,
    pub input: WorkflowNodeInput,
    pub output: WorkflowNodeOutput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowNodeInput {
    pub from_workflow_input: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowNodeOutput {
    pub to_workflow_state: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowEdge {
    pub edge_id: String,
    pub from: String,
    pub to: String,
    pub trigger: WorkflowEdgeTrigger,
    pub event: Option<EventReference>,
    #[serde(default)]
    pub predicate: Option<WorkflowEdgePredicate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowEdgePredicate {
    pub field: String,
    pub equals: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowEdgeTrigger {
    Direct,
    Event,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRegistration {
    pub scope: RegistryScope,
    pub definition: WorkflowDefinition,
    pub workflow_path: String,
    pub registered_at: String,
    pub validator_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRegistryRecord {
    pub scope: RegistryScope,
    pub id: String,
    pub version: String,
    pub lifecycle: Lifecycle,
    pub owner: Owner,
    pub workflow_path: String,
    pub workflow_digest: String,
    pub registered_at: String,
    pub governing_spec: String,
    pub validator_version: String,
    pub evidence: WorkflowRegistrationEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowDiscoveryIndexEntry {
    pub scope: RegistryScope,
    pub id: String,
    pub version: String,
    pub lifecycle: Lifecycle,
    pub owner: Owner,
    pub summary: String,
    pub tags: Vec<String>,
    pub participating_capabilities: Vec<String>,
    pub events_used: Vec<String>,
    pub start_node: String,
    pub terminal_nodes: Vec<String>,
    pub registered_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRegistrationEvidence {
    pub evidence_id: String,
    pub workflow_id: String,
    pub workflow_version: String,
    pub scope: RegistryScope,
    pub governing_spec: String,
    pub validator_version: String,
    pub produced_at: String,
    pub result: WorkflowRegistrationResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowRegistrationResult {
    Passed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRegistrationOutcome {
    pub record: WorkflowRegistryRecord,
    pub index_entry: WorkflowDiscoveryIndexEntry,
    pub evidence: WorkflowRegistrationEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWorkflow {
    pub definition: WorkflowDefinition,
    pub record: WorkflowRegistryRecord,
    pub index_entry: WorkflowDiscoveryIndexEntry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowErrorCode {
    MissingRequiredField,
    InvalidLiteral,
    InvalidSemver,
    DuplicateItem,
    MissingReference,
    EdgeSchemaMismatch,
    InvalidStartNode,
    InvalidTerminalNode,
    InvalidEdgeReference,
    InvalidEventEdge,
    DeterministicCycleNotAllowed,
    ImmutableVersionConflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowError {
    pub code: WorkflowErrorCode,
    pub path: String,
    pub message: String,
    pub severity: ErrorSeverity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowFailure {
    pub errors: Vec<WorkflowError>,
}

#[derive(Debug, Clone, Default)]
pub struct WorkflowRegistry {
    definitions: BTreeMap<(RegistryScope, String, String), WorkflowDefinition>,
    records: BTreeMap<(RegistryScope, String, String), WorkflowRegistryRecord>,
    index: BTreeMap<(RegistryScope, String, String), WorkflowDiscoveryIndexEntry>,
}

impl WorkflowRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one deterministic workflow definition.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowFailure`] when required fields are invalid, workflow
    /// references are missing or incompatible, a deterministic cycle is found,
    /// or an immutable published version would be changed.
    pub fn register(
        &mut self,
        capabilities: &CapabilityRegistry,
        request: WorkflowRegistration,
    ) -> Result<WorkflowRegistrationOutcome, WorkflowFailure> {
        let WorkflowRegistration {
            scope,
            definition,
            workflow_path,
            registered_at,
            validator_version,
        } = request;

        let mut errors = Vec::new();
        validate_workflow_fields(&definition, &workflow_path, &registered_at, &mut errors);
        validate_workflow_references(scope, capabilities, &definition, &mut errors);

        if !errors.is_empty() {
            return Err(WorkflowFailure { errors });
        }

        let key = (scope, definition.id.clone(), definition.version.clone());
        let digest = workflow_digest(&definition);
        let record = WorkflowRegistryRecord {
            scope,
            id: definition.id.clone(),
            version: definition.version.clone(),
            lifecycle: definition.lifecycle.clone(),
            owner: definition.owner.clone(),
            workflow_path,
            workflow_digest: digest.clone(),
            registered_at: registered_at.clone(),
            governing_spec: definition.governing_spec.clone(),
            validator_version: validator_version.clone(),
            evidence: WorkflowRegistrationEvidence {
                evidence_id: format!("wfreg_{}_{}", definition.id, definition.version),
                workflow_id: definition.id.clone(),
                workflow_version: definition.version.clone(),
                scope,
                governing_spec: definition.governing_spec.clone(),
                validator_version,
                produced_at: registered_at.clone(),
                result: WorkflowRegistrationResult::Passed,
            },
        };
        let index_entry = build_workflow_index(scope, &definition, &registered_at);

        if let Some(existing) = self.records.get(&key) {
            let Some(existing_definition) = self.definitions.get(&key) else {
                return Err(single_error(
                    WorkflowErrorCode::ImmutableVersionConflict,
                    "$.id",
                    "existing workflow record is missing its authoritative definition",
                ));
            };
            let Some(existing_index) = self.index.get(&key) else {
                return Err(single_error(
                    WorkflowErrorCode::ImmutableVersionConflict,
                    "$.id",
                    "existing workflow record is missing its discovery index entry",
                ));
            };

            // Idempotent re-registration is driven by content digest and canonical definition,
            // not by validator metadata. If the definition is identical, return the existing
            // record without modifying registry state.
            if existing_definition == &definition {
                return Ok(WorkflowRegistrationOutcome {
                    record: existing.clone(),
                    index_entry: existing_index.clone(),
                    evidence: existing.evidence.clone(),
                });
            }

            return Err(single_error(
                WorkflowErrorCode::ImmutableVersionConflict,
                "$.version",
                "published workflow versions are immutable within a scope",
            ));
        }

        self.definitions.insert(key.clone(), definition);
        self.records.insert(key.clone(), record.clone());
        self.index.insert(key, index_entry.clone());

        Ok(WorkflowRegistrationOutcome {
            evidence: record.evidence.clone(),
            record,
            index_entry,
        })
    }

    #[must_use]
    pub fn find_exact(
        &self,
        lookup_scope: LookupScope,
        id: &str,
        version: &str,
    ) -> Option<ResolvedWorkflow> {
        for &scope in lookup_order(lookup_scope) {
            let key = (scope, id.to_string(), version.to_string());
            if let Some(record) = self.records.get(&key) {
                let definition = self.definitions.get(&key)?.clone();
                let index_entry = self.index.get(&key)?.clone();
                return Some(ResolvedWorkflow {
                    definition,
                    record: record.clone(),
                    index_entry,
                });
            }
        }
        None
    }

    #[must_use]
    pub(crate) fn graph_entries(&self) -> Vec<ResolvedWorkflow> {
        self.records
            .iter()
            .filter_map(|((scope, id, version), record)| {
                let key = (*scope, id.clone(), version.clone());
                let definition = self.definitions.get(&key)?.clone();
                let index_entry = self.index.get(&key)?.clone();
                Some(ResolvedWorkflow {
                    definition,
                    record: record.clone(),
                    index_entry,
                })
            })
            .collect()
    }

    #[must_use]
    pub fn discover(&self, lookup_scope: LookupScope) -> Vec<WorkflowDiscoveryIndexEntry> {
        let mut results = Vec::new();
        let mut shadowed = BTreeSet::new();

        for &scope in lookup_order(lookup_scope) {
            let entries = self
                .index
                .iter()
                .filter(|((entry_scope, _, _), _)| *entry_scope == scope);

            for ((_, id, version), entry) in entries {
                if lookup_scope == LookupScope::PreferPrivate
                    && scope == RegistryScope::Public
                    && shadowed.contains(&(id.clone(), version.clone()))
                {
                    continue;
                }

                if scope == RegistryScope::Private {
                    shadowed.insert((id.clone(), version.clone()));
                }

                results.push(entry.clone());
            }
        }

        results.sort_by(|left, right| {
            left.id.cmp(&right.id).then_with(|| {
                let left_version = Version::parse(&left.version).ok();
                let right_version = Version::parse(&right.version).ok();
                match (left_version, right_version) {
                    (Some(left_version), Some(right_version)) => right_version.cmp(&left_version),
                    _ => right.version.cmp(&left.version),
                }
            })
        });
        results
    }
}

#[must_use]
pub fn workflow_artifact_record(
    workflow_id: &str,
    workflow_version: &str,
    artifact_ref: &str,
) -> CapabilityArtifactRecord {
    CapabilityArtifactRecord {
        artifact_ref: artifact_ref.to_string(),
        implementation_kind: ImplementationKind::Workflow,
        source: crate::SourceReference {
            kind: crate::SourceKind::Local,
            location: format!("workflow://{workflow_id}@{workflow_version}"),
        },
        binary: None,
        workflow_ref: Some(WorkflowReference {
            workflow_id: workflow_id.to_string(),
            workflow_version: workflow_version.to_string(),
        }),
        digests: crate::ArtifactDigests {
            source_digest: format!("workflow:{workflow_id}:{workflow_version}"),
            binary_digest: None,
        },
        provenance: crate::RegistryProvenance {
            source: "workflow-registry".to_string(),
            author: "traverse".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        },
    }
}

#[allow(clippy::too_many_lines)]
fn validate_workflow_fields(
    definition: &WorkflowDefinition,
    workflow_path: &str,
    registered_at: &str,
    errors: &mut Vec<WorkflowError>,
) {
    require_non_empty(&definition.kind, "$.kind", errors);
    require_non_empty(&definition.schema_version, "$.schema_version", errors);
    require_non_empty(&definition.id, "$.id", errors);
    require_non_empty(&definition.name, "$.name", errors);
    require_non_empty(&definition.version, "$.version", errors);
    require_non_empty(&definition.owner.team, "$.owner.team", errors);
    require_non_empty(&definition.owner.contact, "$.owner.contact", errors);
    require_non_empty(&definition.summary, "$.summary", errors);
    require_non_empty(workflow_path, "$.workflow_path", errors);
    require_non_empty(registered_at, "$.registered_at", errors);
    require_non_empty(&definition.start_node, "$.start_node", errors);
    require_non_empty(&definition.governing_spec, "$.governing_spec", errors);

    if definition.kind != WORKFLOW_KIND {
        errors.push(workflow_error(
            WorkflowErrorCode::InvalidLiteral,
            "$.kind",
            "kind must equal workflow_definition",
        ));
    }
    if definition.schema_version != WORKFLOW_SCHEMA_VERSION {
        errors.push(workflow_error(
            WorkflowErrorCode::InvalidLiteral,
            "$.schema_version",
            "schema_version must equal 1.0.0",
        ));
    }
    if definition.governing_spec != WORKFLOW_GOVERNING_SPEC {
        errors.push(workflow_error(
            WorkflowErrorCode::InvalidLiteral,
            "$.governing_spec",
            "governing_spec must equal 007-workflow-registry-traversal",
        ));
    }
    if Version::parse(&definition.version).is_err() {
        errors.push(workflow_error(
            WorkflowErrorCode::InvalidSemver,
            "$.version",
            "version must be valid semantic versioning",
        ));
    }
    if definition.nodes.is_empty() {
        errors.push(workflow_error(
            WorkflowErrorCode::MissingRequiredField,
            "$.nodes",
            "nodes must not be empty",
        ));
    }
    if definition.terminal_nodes.is_empty() {
        errors.push(workflow_error(
            WorkflowErrorCode::InvalidTerminalNode,
            "$.terminal_nodes",
            "terminal_nodes must not be empty",
        ));
    }

    let mut node_ids = BTreeSet::new();
    for (index, node) in definition.nodes.iter().enumerate() {
        let base = format!("$.nodes[{index}]");
        require_non_empty(&node.node_id, &format!("{base}.node_id"), errors);
        require_non_empty(
            &node.capability_id,
            &format!("{base}.capability_id"),
            errors,
        );
        require_non_empty(
            &node.capability_version,
            &format!("{base}.capability_version"),
            errors,
        );
        if !node_ids.insert(node.node_id.clone()) {
            errors.push(workflow_error(
                WorkflowErrorCode::DuplicateItem,
                &format!("{base}.node_id"),
                "node_id values must be unique within one workflow definition",
            ));
        }
        if Version::parse(&node.capability_version).is_err() {
            errors.push(workflow_error(
                WorkflowErrorCode::InvalidSemver,
                &format!("{base}.capability_version"),
                "capability_version must be valid semantic versioning",
            ));
        }
    }

    let mut terminal_ids = BTreeSet::new();
    for (index, terminal) in definition.terminal_nodes.iter().enumerate() {
        if !terminal_ids.insert(terminal.clone()) {
            errors.push(workflow_error(
                WorkflowErrorCode::DuplicateItem,
                &format!("$.terminal_nodes[{index}]"),
                "terminal_nodes must not contain duplicates",
            ));
        }
    }

    let mut edge_ids = BTreeSet::new();
    for (index, edge) in definition.edges.iter().enumerate() {
        let base = format!("$.edges[{index}]");
        require_non_empty(&edge.edge_id, &format!("{base}.edge_id"), errors);
        require_non_empty(&edge.from, &format!("{base}.from"), errors);
        require_non_empty(&edge.to, &format!("{base}.to"), errors);
        if !edge_ids.insert(edge.edge_id.clone()) {
            errors.push(workflow_error(
                WorkflowErrorCode::DuplicateItem,
                &format!("{base}.edge_id"),
                "edge_id values must be unique within one workflow definition",
            ));
        }
        match edge.trigger {
            WorkflowEdgeTrigger::Direct => {
                if edge.event.is_some() {
                    errors.push(workflow_error(
                        WorkflowErrorCode::InvalidEventEdge,
                        &format!("{base}.event"),
                        "direct edges must not declare an event reference",
                    ));
                }
                if edge.predicate.is_some() {
                    errors.push(workflow_error(
                        WorkflowErrorCode::InvalidEventEdge,
                        &format!("{base}.predicate"),
                        "direct edges must not declare an event predicate",
                    ));
                }
            }
            WorkflowEdgeTrigger::Event => {
                let Some(event) = edge.event.as_ref() else {
                    errors.push(workflow_error(
                        WorkflowErrorCode::InvalidEventEdge,
                        &format!("{base}.event"),
                        "event edges must declare an event reference",
                    ));
                    continue;
                };
                if event.event_id.trim().is_empty() {
                    errors.push(workflow_error(
                        WorkflowErrorCode::InvalidEventEdge,
                        &format!("{base}.event.event_id"),
                        "event_id must be non-empty",
                    ));
                }
                if Version::parse(&event.version).is_err() {
                    errors.push(workflow_error(
                        WorkflowErrorCode::InvalidSemver,
                        &format!("{base}.event.version"),
                        "event version must be valid semantic versioning",
                    ));
                }
                if let Some(predicate) = edge.predicate.as_ref()
                    && predicate.field.trim().is_empty()
                {
                    errors.push(workflow_error(
                        WorkflowErrorCode::InvalidEventEdge,
                        &format!("{base}.predicate.field"),
                        "event predicate field must be non-empty",
                    ));
                }
            }
        }
    }

    if !node_ids.contains(&definition.start_node) {
        errors.push(workflow_error(
            WorkflowErrorCode::InvalidStartNode,
            "$.start_node",
            "start_node must reference a declared node",
        ));
    }
    for (index, terminal) in definition.terminal_nodes.iter().enumerate() {
        if !node_ids.contains(terminal) {
            errors.push(workflow_error(
                WorkflowErrorCode::InvalidTerminalNode,
                &format!("$.terminal_nodes[{index}]"),
                "terminal_nodes must reference declared node ids",
            ));
        }
    }
    for (index, edge) in definition.edges.iter().enumerate() {
        if !node_ids.contains(&edge.from) {
            errors.push(workflow_error(
                WorkflowErrorCode::InvalidEdgeReference,
                &format!("$.edges[{index}].from"),
                "edge source must reference a declared node",
            ));
        }
        if !node_ids.contains(&edge.to) {
            errors.push(workflow_error(
                WorkflowErrorCode::InvalidEdgeReference,
                &format!("$.edges[{index}].to"),
                "edge target must reference a declared node",
            ));
        }
    }
    if has_cycle(definition) {
        errors.push(workflow_error(
            WorkflowErrorCode::DeterministicCycleNotAllowed,
            "$.edges",
            "deterministic workflow cycles are not allowed in v0.1",
        ));
    }
}

fn validate_workflow_references(
    scope: RegistryScope,
    capabilities: &CapabilityRegistry,
    definition: &WorkflowDefinition,
    errors: &mut Vec<WorkflowError>,
) {
    let lookup_scope = match scope {
        RegistryScope::Public => LookupScope::PublicOnly,
        RegistryScope::Private => LookupScope::PreferPrivate,
    };

    let node_capabilities = definition
        .nodes
        .iter()
        .map(|node| {
            let resolved = capabilities.find_exact(
                lookup_scope,
                &node.capability_id,
                &node.capability_version,
            );
            (node, resolved)
        })
        .collect::<Vec<_>>();

    for (index, (_node, resolved)) in node_capabilities.iter().enumerate() {
        let Some(_capability) = resolved else {
            errors.push(workflow_error(
                WorkflowErrorCode::MissingReference,
                &format!("$.nodes[{index}]"),
                "workflow node must reference one registered capability contract version",
            ));
            continue;
        };
    }

    let resolved_by_node = node_capabilities
        .into_iter()
        .filter_map(|(node, resolved)| {
            resolved.map(|capability| (node.node_id.clone(), capability))
        })
        .collect::<BTreeMap<_, _>>();

    // Validate that node input/output field selection is internally consistent and
    // schema-compatible in deterministic topological order.
    validate_schema_compatibility(definition, &resolved_by_node, errors);

    let mut direct_edges = BTreeMap::<&str, usize>::new();
    let mut event_edges = BTreeMap::<&str, usize>::new();
    for (index, edge) in definition.edges.iter().enumerate() {
        match edge.trigger {
            WorkflowEdgeTrigger::Direct => {
                *direct_edges.entry(&edge.from).or_default() += 1;
                if direct_edges[edge.from.as_str()] > 1 {
                    errors.push(workflow_error(
                        WorkflowErrorCode::DuplicateItem,
                        &format!("$.edges[{index}]"),
                        "nodes must not declare more than one direct outgoing edge in v0.1",
                    ));
                }
            }
            WorkflowEdgeTrigger::Event => {
                *event_edges.entry(&edge.from).or_default() += 1;
                if event_edges[edge.from.as_str()] > 1 {
                    errors.push(workflow_error(
                        WorkflowErrorCode::DuplicateItem,
                        &format!("$.edges[{index}]"),
                        "nodes must not declare more than one event outgoing edge in v0.1",
                    ));
                }

                let Some(event) = edge.event.as_ref() else {
                    continue;
                };
                let Some(source_capability) = resolved_by_node.get(&edge.from) else {
                    continue;
                };
                let emits_event = source_capability
                    .contract
                    .emits
                    .iter()
                    .any(|declared| declared == event);
                if !emits_event {
                    errors.push(workflow_error(
                        WorkflowErrorCode::InvalidEventEdge,
                        &format!("$.edges[{index}].event"),
                        "event edge must reference an event emitted by the source capability contract",
                    ));
                }
            }
        }
    }
}

fn validate_schema_compatibility(
    definition: &WorkflowDefinition,
    resolved_by_node: &BTreeMap<String, crate::ResolvedCapability>,
    errors: &mut Vec<WorkflowError>,
) {
    // Only validate field-level flow when the workflow declares explicit input properties.
    // Open-schema workflows ({"type":"object"} with no "properties") opt out of strict checking.
    if !schema_declares_properties(&definition.inputs.schema) {
        return;
    }

    let Some(order) = topological_node_order(definition) else {
        return;
    };

    let workflow_input_types = schema_property_types(&definition.inputs.schema);
    let mut state_types = workflow_input_types.clone();

    for node_index in order {
        let node = &definition.nodes[node_index];
        let Some(capability) = resolved_by_node.get(&node.node_id) else {
            continue;
        };

        // Skip field-level input checking when the capability input schema is open.
        let cap_input_schema = &capability.contract.inputs.schema;
        let input_types = if schema_declares_properties(cap_input_schema) {
            schema_property_types(cap_input_schema)
        } else {
            BTreeMap::new()
        };

        let cap_input_has_properties = schema_declares_properties(cap_input_schema);
        for (input_index, key) in node.input.from_workflow_input.iter().enumerate() {
            let Some(state_type) = state_types.get(key) else {
                errors.push(workflow_error(
                    WorkflowErrorCode::EdgeSchemaMismatch,
                    &format!("$.nodes[{node_index}].input.from_workflow_input[{input_index}]"),
                    "workflow input field is not available from prior nodes or workflow inputs",
                ));
                continue;
            };

            // Skip type-checking when the capability input schema is open.
            if !cap_input_has_properties {
                continue;
            }

            let Some(expected_type) = input_types.get(key) else {
                errors.push(workflow_error(
                    WorkflowErrorCode::EdgeSchemaMismatch,
                    &format!("$.nodes[{node_index}].input.from_workflow_input[{input_index}]"),
                    "capability input schema is missing referenced field",
                ));
                continue;
            };

            if expected_type != state_type {
                errors.push(workflow_error(
                    WorkflowErrorCode::EdgeSchemaMismatch,
                    &format!(
                        "$.nodes[{node_index}].input.from_workflow_input[{input_index}]"
                    ),
                    &format!(
                        "field type mismatch: state provides {state_type}, capability expects {expected_type}"
                    ),
                ));
            }
        }

        // Skip output field checking when the capability output schema is open.
        let cap_output_schema = &capability.contract.outputs.schema;
        if schema_declares_properties(cap_output_schema) {
            let output_types = schema_property_types(cap_output_schema);
            for (output_index, key) in node.output.to_workflow_state.iter().enumerate() {
                let Some(value_type) = output_types.get(key) else {
                    errors.push(workflow_error(
                        WorkflowErrorCode::EdgeSchemaMismatch,
                        &format!("$.nodes[{node_index}].output.to_workflow_state[{output_index}]"),
                        "capability output schema is missing referenced field",
                    ));
                    continue;
                };
                state_types.insert(key.clone(), value_type.clone());
            }
        } else {
            // Open output schema: add all declared state keys with an inferred type.
            for key in &node.output.to_workflow_state {
                state_types.insert(key.clone(), "object".to_string());
            }
        }
    }
}

/// Returns true only when the schema is an object schema with an explicit `properties` map.
/// Schemas that lack an explicit `properties` key are treated as open / schema-less, so
/// field-level validation is skipped for them.
fn schema_declares_properties(schema: &Value) -> bool {
    let Value::Object(root) = schema else {
        return false;
    };
    matches!(root.get("type"), Some(Value::String(t)) if t == "object")
        && root.get("properties").is_some()
}

fn schema_property_types(schema: &Value) -> BTreeMap<String, String> {
    let Value::Object(root) = schema else {
        return BTreeMap::new();
    };
    let Some(Value::String(schema_type)) = root.get("type") else {
        return BTreeMap::new();
    };
    if schema_type != "object" {
        return BTreeMap::new();
    }
    let Some(Value::Object(properties)) = root.get("properties") else {
        return BTreeMap::new();
    };

    let mut types = BTreeMap::new();
    for (key, value) in properties {
        let Value::Object(property) = value else {
            continue;
        };
        let Some(Value::String(property_type)) = property.get("type") else {
            continue;
        };
        types.insert(key.clone(), property_type.clone());
    }
    types
}

fn topological_node_order(definition: &WorkflowDefinition) -> Option<Vec<usize>> {
    let id_to_index: BTreeMap<&str, usize> = definition
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.node_id.as_str(), i))
        .collect();

    let n = definition.nodes.len();
    let mut indegree = vec![0_usize; n];
    let mut adjacency = vec![Vec::<usize>::new(); n];

    for edge in &definition.edges {
        if let (Some(&fi), Some(&ti)) = (
            id_to_index.get(edge.from.as_str()),
            id_to_index.get(edge.to.as_str()),
        ) {
            adjacency[fi].push(ti);
            indegree[ti] += 1;
        }
    }

    let mut available: Vec<usize> = (0..n).filter(|&i| indegree[i] == 0).collect();
    available.sort_by_key(|&i| &definition.nodes[i].node_id);

    let mut order = Vec::new();
    while let Some(next) = available.first().copied() {
        available.remove(0);
        order.push(next);
        for &neighbor in &adjacency[next] {
            indegree[neighbor] = indegree[neighbor].saturating_sub(1);
            if indegree[neighbor] == 0 {
                available.push(neighbor);
                available.sort_by_key(|&i| &definition.nodes[i].node_id);
            }
        }
    }

    if order.len() != n {
        return None;
    }

    Some(order)
}

fn build_workflow_index(
    scope: RegistryScope,
    definition: &WorkflowDefinition,
    registered_at: &str,
) -> WorkflowDiscoveryIndexEntry {
    WorkflowDiscoveryIndexEntry {
        scope,
        id: definition.id.clone(),
        version: definition.version.clone(),
        lifecycle: definition.lifecycle.clone(),
        owner: definition.owner.clone(),
        summary: definition.summary.clone(),
        tags: dedup_strings(definition.tags.clone()),
        participating_capabilities: dedup_strings(
            definition
                .nodes
                .iter()
                .map(|node| node.capability_id.clone())
                .collect(),
        ),
        events_used: dedup_strings(
            definition
                .edges
                .iter()
                .filter_map(|edge| {
                    edge.event
                        .as_ref()
                        .map(|event| format!("{}@{}", event.event_id, event.version))
                })
                .collect(),
        ),
        start_node: definition.start_node.clone(),
        terminal_nodes: dedup_strings(definition.terminal_nodes.clone()),
        registered_at: registered_at.to_string(),
    }
}

fn dedup_strings(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn has_cycle(definition: &WorkflowDefinition) -> bool {
    let mut adjacency = BTreeMap::<String, Vec<String>>::new();
    for edge in &definition.edges {
        adjacency
            .entry(edge.from.clone())
            .or_default()
            .push(edge.to.clone());
    }

    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for node in definition.nodes.iter().map(|node| node.node_id.clone()) {
        if dfs_cycle(&node, &adjacency, &mut visiting, &mut visited) {
            return true;
        }
    }
    false
}

fn dfs_cycle(
    node: &str,
    adjacency: &BTreeMap<String, Vec<String>>,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
) -> bool {
    if visited.contains(node) {
        return false;
    }
    if !visiting.insert(node.to_string()) {
        return true;
    }
    if let Some(next_nodes) = adjacency.get(node) {
        for next in next_nodes {
            if dfs_cycle(next, adjacency, visiting, visited) {
                return true;
            }
        }
    }
    visiting.remove(node);
    visited.insert(node.to_string());
    false
}

fn workflow_digest(definition: &WorkflowDefinition) -> String {
    let mut artifact = Map::new();
    artifact.insert("kind".to_string(), Value::String(definition.kind.clone()));
    artifact.insert(
        "schema_version".to_string(),
        Value::String(definition.schema_version.clone()),
    );
    artifact.insert("id".to_string(), Value::String(definition.id.clone()));
    artifact.insert("name".to_string(), Value::String(definition.name.clone()));
    artifact.insert(
        "version".to_string(),
        Value::String(definition.version.clone()),
    );
    artifact.insert(
        "lifecycle".to_string(),
        Value::String(format!("{:?}", definition.lifecycle)),
    );
    artifact.insert(
        "owner".to_string(),
        json!({
            "team": definition.owner.team,
            "contact": definition.owner.contact,
        }),
    );
    artifact.insert(
        "summary".to_string(),
        Value::String(definition.summary.clone()),
    );
    artifact.insert("inputs".to_string(), definition.inputs.schema.clone());
    artifact.insert("outputs".to_string(), definition.outputs.schema.clone());
    artifact.insert(
        "nodes".to_string(),
        Value::Array(
            definition
                .nodes
                .iter()
                .map(|node| {
                    json!({
                        "node_id": node.node_id,
                        "capability_id": node.capability_id,
                        "capability_version": node.capability_version,
                        "input": node.input.from_workflow_input,
                        "output": node.output.to_workflow_state,
                    })
                })
                .collect(),
        ),
    );
    artifact.insert(
        "edges".to_string(),
        Value::Array(
            definition
                .edges
                .iter()
                .map(|edge| {
                    json!({
                        "edge_id": edge.edge_id,
                        "from": edge.from,
                        "to": edge.to,
                        "trigger": match edge.trigger {
                            WorkflowEdgeTrigger::Direct => "direct",
                            WorkflowEdgeTrigger::Event => "event",
                        },
                        "event": edge.event.as_ref().map(|event| {
                            json!({
                                "event_id": event.event_id,
                                "version": event.version,
                            })
                        }),
                    })
                })
                .collect(),
        ),
    );
    artifact.insert(
        "start_node".to_string(),
        Value::String(definition.start_node.clone()),
    );
    artifact.insert(
        "terminal_nodes".to_string(),
        Value::Array(
            definition
                .terminal_nodes
                .iter()
                .cloned()
                .map(Value::String)
                .collect(),
        ),
    );
    artifact.insert(
        "tags".to_string(),
        Value::Array(definition.tags.iter().cloned().map(Value::String).collect()),
    );
    artifact.insert(
        "governing_spec".to_string(),
        Value::String(definition.governing_spec.clone()),
    );

    format!("workflow:{}", Value::Object(artifact))
}

fn require_non_empty(value: &str, path: &str, errors: &mut Vec<WorkflowError>) {
    if value.trim().is_empty() {
        errors.push(workflow_error(
            WorkflowErrorCode::MissingRequiredField,
            path,
            "field must be non-empty",
        ));
    }
}

fn single_error(code: WorkflowErrorCode, path: &str, message: &str) -> WorkflowFailure {
    WorkflowFailure {
        errors: vec![workflow_error(code, path, message)],
    }
}

fn workflow_error(code: WorkflowErrorCode, path: &str, message: &str) -> WorkflowError {
    WorkflowError {
        code,
        path: path.to_string(),
        message: message.to_string(),
        severity: ErrorSeverity::Error,
    }
}

fn lookup_order(lookup_scope: LookupScope) -> &'static [RegistryScope] {
    match lookup_scope {
        LookupScope::PublicOnly => &[RegistryScope::Public],
        LookupScope::PreferPrivate => &[RegistryScope::Private, RegistryScope::Public],
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::{
        ArtifactDigests, BinaryFormat, BinaryReference, CapabilityRegistration,
        ComposabilityMetadata, CompositionKind, CompositionPattern, RegistryProvenance, SourceKind,
        SourceReference,
    };
    use serde_json::json;
    use traverse_contracts::{
        BinaryFormat as ContractBinaryFormat, Condition, Entrypoint, EntrypointKind,
        EvidenceStatus, EvidenceType, Execution, ExecutionConstraints, ExecutionTarget,
        FilesystemAccess, HostApiAccess, NetworkAccess, Provenance, ProvenanceSource, ServiceType,
        SideEffect, SideEffectKind, ValidationEvidence,
    };

    #[test]
    fn registers_valid_workflow_and_supports_private_overlay_lookup() {
        let capabilities = capability_registry();
        let mut registry = WorkflowRegistry::new();

        let public = register_workflow_ok(
            &mut registry,
            &capabilities,
            WorkflowRegistration {
                scope: RegistryScope::Public,
                definition: valid_workflow_definition(),
                workflow_path: "workflows/public.json".to_string(),
                registered_at: "2026-03-27T00:00:00Z".to_string(),
                validator_version: "workflow-validator/0.1.0".to_string(),
            },
        );
        let private = register_workflow_ok(
            &mut registry,
            &capabilities,
            WorkflowRegistration {
                scope: RegistryScope::Private,
                definition: WorkflowDefinition {
                    summary: "private summary".to_string(),
                    ..valid_workflow_definition()
                },
                workflow_path: "workflows/private.json".to_string(),
                registered_at: "2026-03-27T00:01:00Z".to_string(),
                validator_version: "workflow-validator/0.1.0".to_string(),
            },
        );

        assert_eq!(public.record.id, "content.comments.publish-comment");
        let resolved = find_workflow_exact(
            &registry,
            LookupScope::PreferPrivate,
            "content.comments.publish-comment",
            "1.0.0",
        );
        assert_eq!(resolved.record.scope, RegistryScope::Private);
        assert_eq!(resolved.record.workflow_path, "workflows/private.json");
        assert_eq!(private.index_entry.summary, "private summary");
    }

    #[test]
    fn discover_covers_public_only_and_private_overlay_paths() {
        let capabilities = capability_registry();
        let mut registry = WorkflowRegistry::new();

        register_workflow_ok(
            &mut registry,
            &capabilities,
            WorkflowRegistration {
                scope: RegistryScope::Public,
                definition: valid_workflow_definition(),
                workflow_path: "workflows/public.json".to_string(),
                registered_at: "2026-03-27T00:00:00Z".to_string(),
                validator_version: "workflow-validator/0.1.0".to_string(),
            },
        );
        register_workflow_ok(
            &mut registry,
            &capabilities,
            WorkflowRegistration {
                scope: RegistryScope::Private,
                definition: WorkflowDefinition {
                    summary: "private summary".to_string(),
                    ..valid_workflow_definition()
                },
                workflow_path: "workflows/private.json".to_string(),
                registered_at: "2026-03-27T00:01:00Z".to_string(),
                validator_version: "workflow-validator/0.1.0".to_string(),
            },
        );
        register_workflow_ok(
            &mut registry,
            &capabilities,
            WorkflowRegistration {
                scope: RegistryScope::Public,
                definition: WorkflowDefinition {
                    version: "1.1.0".to_string(),
                    ..valid_workflow_definition()
                },
                workflow_path: "workflows/public-v1-1-0.json".to_string(),
                registered_at: "2026-03-27T00:02:00Z".to_string(),
                validator_version: "workflow-validator/0.1.0".to_string(),
            },
        );
        registry.index.insert(
            (
                RegistryScope::Public,
                "content.comments.publish-comment".to_string(),
                "invalid".to_string(),
            ),
            WorkflowDiscoveryIndexEntry {
                scope: RegistryScope::Public,
                id: "content.comments.publish-comment".to_string(),
                version: "invalid".to_string(),
                lifecycle: Lifecycle::Active,
                owner: Owner {
                    team: "comments".to_string(),
                    contact: "comments@example.com".to_string(),
                },
                summary: "invalid semver workflow".to_string(),
                tags: vec!["comments".to_string()],
                participating_capabilities: vec![
                    "content.comments.create-comment-draft".to_string(),
                ],
                events_used: vec!["content.comments.draft-created".to_string()],
                start_node: "create_draft".to_string(),
                terminal_nodes: vec!["persist_comment".to_string()],
                registered_at: "2026-03-27T00:03:00Z".to_string(),
            },
        );

        let public_only = registry.discover(LookupScope::PublicOnly);
        let prefer_private = registry.discover(LookupScope::PreferPrivate);

        assert_eq!(public_only.len(), 3);
        assert_eq!(public_only[0].version, "invalid");
        assert_eq!(public_only[1].version, "1.1.0");
        assert_eq!(prefer_private.len(), 3);
        assert!(prefer_private.iter().any(|entry| {
            entry.scope == RegistryScope::Private && entry.summary == "private summary"
        }));
        assert!(
            prefer_private
                .iter()
                .any(|entry| entry.version == "invalid")
        );
        assert!(prefer_private.iter().any(|entry| entry.version == "1.1.0"));
    }

    #[test]
    fn registers_idempotently_when_definition_digest_matches_even_if_metadata_differs() {
        let capabilities = capability_registry();
        let mut registry = WorkflowRegistry::new();

        register_workflow_ok(
            &mut registry,
            &capabilities,
            WorkflowRegistration {
                scope: RegistryScope::Public,
                definition: valid_workflow_definition(),
                workflow_path: "workflows/original.json".to_string(),
                registered_at: "2026-03-27T00:00:00Z".to_string(),
                validator_version: "workflow-validator/0.1.0".to_string(),
            },
        );

        let outcome = registry
            .register(
                &capabilities,
                WorkflowRegistration {
                    scope: RegistryScope::Public,
                    definition: valid_workflow_definition(),
                    workflow_path: "workflows/changed.json".to_string(),
                    registered_at: "2026-03-27T01:00:00Z".to_string(),
                    validator_version: "workflow-validator/9.9.9".to_string(),
                },
            )
            .expect("idempotent registration must succeed");

        assert_eq!(outcome.record.workflow_path, "workflows/original.json");
    }

    #[test]
    fn rejects_workflow_with_schema_mismatch_when_required_field_is_unavailable() {
        let capabilities = capability_registry();
        let mut registry = WorkflowRegistry::new();
        let mut definition = valid_workflow_definition();
        definition.nodes[1]
            .input
            .from_workflow_input
            .push("missing".to_string());

        let failure = registry
            .register(
                &capabilities,
                WorkflowRegistration {
                    scope: RegistryScope::Public,
                    definition,
                    workflow_path: "workflows/bad.json".to_string(),
                    registered_at: "2026-03-27T00:00:00Z".to_string(),
                    validator_version: "workflow-validator/0.1.0".to_string(),
                },
            )
            .expect_err("schema mismatch must fail");

        assert!(
            failure
                .errors
                .iter()
                .any(|error| error.code == WorkflowErrorCode::EdgeSchemaMismatch)
        );
    }

    #[test]
    fn rejects_invalid_workflow_fields_references_and_cycles() {
        let capabilities = capability_registry();
        let mut registry = WorkflowRegistry::new();
        let mut definition = valid_workflow_definition();
        definition.kind = "bad".to_string();
        definition.schema_version = "2.0.0".to_string();
        definition.version = "oops".to_string();
        definition.owner.team.clear();
        definition.start_node = "missing".to_string();
        definition.terminal_nodes = vec!["missing".to_string(), "missing".to_string()];
        definition.nodes[0].capability_version = "oops".to_string();
        definition.nodes.push(definition.nodes[0].clone());
        definition.edges.push(WorkflowEdge {
            edge_id: "edge_1".to_string(),
            from: "validate_comment".to_string(),
            to: "create_draft".to_string(),
            trigger: WorkflowEdgeTrigger::Direct,
            event: Some(EventReference {
                event_id: "content.comments.draft-created".to_string(),
                version: "1.0.0".to_string(),
            }),
            predicate: None,
        });

        let failure = register_workflow_err(
            &mut registry,
            &capabilities,
            WorkflowRegistration {
                scope: RegistryScope::Public,
                definition,
                workflow_path: String::new(),
                registered_at: String::new(),
                validator_version: "validator".to_string(),
            },
        );

        assert!(
            failure
                .errors
                .iter()
                .any(|error| error.code == WorkflowErrorCode::InvalidLiteral)
        );
        assert!(
            failure
                .errors
                .iter()
                .any(|error| error.code == WorkflowErrorCode::InvalidSemver)
        );
        assert!(
            failure
                .errors
                .iter()
                .any(|error| error.code == WorkflowErrorCode::DuplicateItem)
        );
        assert!(
            failure
                .errors
                .iter()
                .any(|error| error.code == WorkflowErrorCode::InvalidStartNode)
        );
        assert!(
            failure
                .errors
                .iter()
                .any(|error| error.code == WorkflowErrorCode::InvalidTerminalNode)
        );
        assert!(
            failure
                .errors
                .iter()
                .any(|error| error.code == WorkflowErrorCode::DeterministicCycleNotAllowed)
        );
    }

    #[test]
    fn rejects_missing_capabilities_and_invalid_event_edges() {
        let capabilities = capability_registry();
        let mut registry = WorkflowRegistry::new();
        let mut definition = valid_workflow_definition();
        definition.nodes[0].capability_id = "missing".to_string();
        definition.edges[1].event = Some(EventReference {
            event_id: "content.comments.other".to_string(),
            version: "1.0.0".to_string(),
        });
        definition.edges.push(WorkflowEdge {
            edge_id: "duplicate_event".to_string(),
            from: "validate_comment".to_string(),
            to: "persist_comment".to_string(),
            trigger: WorkflowEdgeTrigger::Event,
            event: Some(EventReference {
                event_id: "content.comments.validated".to_string(),
                version: "1.0.0".to_string(),
            }),
            predicate: None,
        });

        let failure = register_workflow_err(
            &mut registry,
            &capabilities,
            WorkflowRegistration {
                scope: RegistryScope::Public,
                definition,
                workflow_path: "workflows/public.json".to_string(),
                registered_at: "2026-03-27T00:00:00Z".to_string(),
                validator_version: "validator".to_string(),
            },
        );

        assert!(
            failure
                .errors
                .iter()
                .any(|error| error.code == WorkflowErrorCode::MissingReference)
        );
        assert!(
            failure
                .errors
                .iter()
                .any(|error| error.code == WorkflowErrorCode::InvalidEventEdge)
        );
        assert!(
            failure
                .errors
                .iter()
                .any(|error| error.message.contains("more than one event outgoing edge"))
        );
    }

    #[test]
    fn preserves_immutable_published_workflow_versions() {
        let capabilities = capability_registry();
        let mut registry = WorkflowRegistry::new();
        let request = WorkflowRegistration {
            scope: RegistryScope::Public,
            definition: valid_workflow_definition(),
            workflow_path: "workflows/public.json".to_string(),
            registered_at: "2026-03-27T00:00:00Z".to_string(),
            validator_version: "validator".to_string(),
        };
        let first = register_workflow_ok(&mut registry, &capabilities, request.clone());
        let second = register_workflow_ok(&mut registry, &capabilities, request);
        assert_eq!(first.record, second.record);

        let failure = register_workflow_err(
            &mut registry,
            &capabilities,
            WorkflowRegistration {
                definition: WorkflowDefinition {
                    summary: "changed summary".to_string(),
                    ..valid_workflow_definition()
                },
                workflow_path: "workflows/changed.json".to_string(),
                registered_at: "2026-03-27T00:00:01Z".to_string(),
                validator_version: "validator".to_string(),
                scope: RegistryScope::Public,
            },
        );
        assert_eq!(
            failure.errors[0].code,
            WorkflowErrorCode::ImmutableVersionConflict
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn covers_internal_registry_inconsistency_and_exact_lookup_miss_paths() {
        let capabilities = capability_registry();
        let request = WorkflowRegistration {
            scope: RegistryScope::Public,
            definition: valid_workflow_definition(),
            workflow_path: "workflows/public.json".to_string(),
            registered_at: "2026-03-27T00:00:00Z".to_string(),
            validator_version: "validator".to_string(),
        };
        let mut registry = WorkflowRegistry::new();
        let key = (
            RegistryScope::Public,
            request.definition.id.clone(),
            request.definition.version.clone(),
        );
        let record = WorkflowRegistryRecord {
            scope: RegistryScope::Public,
            id: request.definition.id.clone(),
            version: request.definition.version.clone(),
            lifecycle: Lifecycle::Active,
            owner: request.definition.owner.clone(),
            workflow_path: "workflows/public.json".to_string(),
            workflow_digest: "digest".to_string(),
            registered_at: "2026-03-27T00:00:00Z".to_string(),
            governing_spec: WORKFLOW_GOVERNING_SPEC.to_string(),
            validator_version: "validator".to_string(),
            evidence: WorkflowRegistrationEvidence {
                evidence_id: "evidence".to_string(),
                workflow_id: request.definition.id.clone(),
                workflow_version: request.definition.version.clone(),
                scope: RegistryScope::Public,
                governing_spec: WORKFLOW_GOVERNING_SPEC.to_string(),
                validator_version: "validator".to_string(),
                produced_at: "2026-03-27T00:00:00Z".to_string(),
                result: WorkflowRegistrationResult::Passed,
            },
        };
        registry.records.insert(key.clone(), record.clone());
        let failure = register_workflow_err(&mut registry, &capabilities, request.clone());
        assert!(
            failure
                .errors
                .iter()
                .any(|error| error.message.contains("authoritative definition"))
        );

        let mut registry = WorkflowRegistry::new();
        registry.records.insert(key.clone(), record);
        registry
            .definitions
            .insert(key.clone(), request.definition.clone());
        let failure = register_workflow_err(&mut registry, &capabilities, request);
        assert!(
            failure
                .errors
                .iter()
                .any(|error| error.message.contains("discovery index entry"))
        );

        let registry = WorkflowRegistry::new();
        assert!(
            registry
                .find_exact(LookupScope::PublicOnly, "missing", "1.0.0")
                .is_none()
        );

        let mut registry = WorkflowRegistry::new();
        register_workflow_ok(
            &mut registry,
            &capabilities,
            WorkflowRegistration {
                scope: RegistryScope::Public,
                definition: WorkflowDefinition {
                    edges: vec![WorkflowEdge {
                        edge_id: "direct".to_string(),
                        from: "create_draft".to_string(),
                        to: "persist_comment".to_string(),
                        trigger: WorkflowEdgeTrigger::Direct,
                        event: None,
                        predicate: None,
                    }],
                    ..valid_workflow_definition()
                },
                workflow_path: "workflows/direct.json".to_string(),
                registered_at: "2026-03-27T00:02:00Z".to_string(),
                validator_version: "validator".to_string(),
            },
        );
        // Re-registration with the same definition content but different validator metadata is
        // idempotent: the existing record is returned unchanged (spec 041 behaviour).
        let outcome = register_workflow_ok(
            &mut registry,
            &capabilities,
            WorkflowRegistration {
                scope: RegistryScope::Public,
                definition: WorkflowDefinition {
                    edges: vec![WorkflowEdge {
                        edge_id: "direct".to_string(),
                        from: "create_draft".to_string(),
                        to: "persist_comment".to_string(),
                        trigger: WorkflowEdgeTrigger::Direct,
                        event: None,
                        predicate: None,
                    }],
                    ..valid_workflow_definition()
                },
                workflow_path: "workflows/direct-metadata-only.json".to_string(),
                registered_at: "2026-03-27T00:03:00Z".to_string(),
                validator_version: "validator".to_string(),
            },
        );
        // Idempotent re-registration returns the ORIGINAL record (original path preserved).
        assert_eq!(outcome.record.workflow_path, "workflows/direct.json");
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn rejects_additional_invalid_shapes_and_non_runtime_references() {
        let capabilities = capability_registry();
        let mut registry = WorkflowRegistry::new();
        let mut definition = valid_workflow_definition();
        definition.governing_spec = "wrong".to_string();
        definition.nodes = vec![WorkflowNode {
            node_id: String::new(),
            capability_id: String::new(),
            capability_version: String::new(),
            input: WorkflowNodeInput {
                from_workflow_input: vec![],
            },
            output: WorkflowNodeOutput {
                to_workflow_state: vec![],
            },
        }];
        definition.terminal_nodes = Vec::new();
        definition.edges = vec![
            WorkflowEdge {
                edge_id: "edge".to_string(),
                from: "missing".to_string(),
                to: "missing".to_string(),
                trigger: WorkflowEdgeTrigger::Direct,
                event: Some(EventReference {
                    event_id: "content.comments.draft-created".to_string(),
                    version: "1.0.0".to_string(),
                }),
                predicate: Some(WorkflowEdgePredicate {
                    field: "payload.severity".to_string(),
                    equals: json!("normal"),
                }),
            },
            WorkflowEdge {
                edge_id: "edge".to_string(),
                from: String::new(),
                to: String::new(),
                trigger: WorkflowEdgeTrigger::Event,
                event: None,
                predicate: None,
            },
            WorkflowEdge {
                edge_id: "edge-3".to_string(),
                from: "missing".to_string(),
                to: "missing".to_string(),
                trigger: WorkflowEdgeTrigger::Event,
                event: Some(EventReference {
                    event_id: String::new(),
                    version: "bad".to_string(),
                }),
                predicate: Some(WorkflowEdgePredicate {
                    field: String::new(),
                    equals: json!(true),
                }),
            },
        ];
        let failure = register_workflow_err(
            &mut registry,
            &capabilities,
            WorkflowRegistration {
                scope: RegistryScope::Public,
                definition,
                workflow_path: "workflows/bad.json".to_string(),
                registered_at: "2026-03-27T00:00:00Z".to_string(),
                validator_version: "validator".to_string(),
            },
        );
        assert!(
            failure
                .errors
                .iter()
                .any(|error| error.message.contains("governing_spec"))
        );
        assert!(
            failure
                .errors
                .iter()
                .any(|error| error.message.contains("nodes must not be empty")
                    || error.path.contains("$.nodes[0].node_id"))
        );
        assert!(
            failure
                .errors
                .iter()
                .any(|error| error.message.contains("terminal_nodes must not be empty"))
        );
        assert!(failure.errors.iter().any(|error| {
            error
                .message
                .contains("direct edges must not declare an event reference")
        }));
        assert!(failure.errors.iter().any(|error| {
            error
                .message
                .contains("direct edges must not declare an event predicate")
        }));
        assert!(failure.errors.iter().any(|error| {
            error
                .message
                .contains("event edges must declare an event reference")
        }));
        assert!(
            failure
                .errors
                .iter()
                .any(|error| error.message.contains("event_id must be non-empty"))
        );
        assert!(failure.errors.iter().any(|error| {
            error
                .message
                .contains("event predicate field must be non-empty")
        }));
        assert!(failure.errors.iter().any(|error| {
            error
                .message
                .contains("edge source must reference a declared node")
        }));

        let mut archived_capabilities = capability_registry();
        let archived = capability_registration(
            "content.comments.archived-step",
            vec![EventReference {
                event_id: "content.comments.archived".to_string(),
                version: "1.0.0".to_string(),
            }],
        );
        let mut archived_contract = archived.contract.clone();
        archived_contract.lifecycle = Lifecycle::Archived;
        register_capability_ok(
            &mut archived_capabilities,
            CapabilityRegistration {
                contract: archived_contract,
                ..archived
            },
        );
        let mut registry = WorkflowRegistry::new();
        let failure = register_workflow_err(
            &mut registry,
            &archived_capabilities,
            WorkflowRegistration {
                scope: RegistryScope::Public,
                definition: WorkflowDefinition {
                    nodes: vec![WorkflowNode {
                        node_id: "archived".to_string(),
                        capability_id: "content.comments.archived-step".to_string(),
                        capability_version: "1.0.0".to_string(),
                        input: WorkflowNodeInput {
                            from_workflow_input: vec![],
                        },
                        output: WorkflowNodeOutput {
                            to_workflow_state: vec![],
                        },
                    }],
                    edges: vec![
                        WorkflowEdge {
                            edge_id: "d1".to_string(),
                            from: "archived".to_string(),
                            to: "archived".to_string(),
                            trigger: WorkflowEdgeTrigger::Direct,
                            event: None,
                            predicate: None,
                        },
                        WorkflowEdge {
                            edge_id: "d2".to_string(),
                            from: "archived".to_string(),
                            to: "archived".to_string(),
                            trigger: WorkflowEdgeTrigger::Direct,
                            event: None,
                            predicate: None,
                        },
                    ],
                    start_node: "archived".to_string(),
                    terminal_nodes: vec!["archived".to_string()],
                    ..valid_workflow_definition()
                },
                workflow_path: "workflows/archived.json".to_string(),
                registered_at: "2026-03-27T00:00:00Z".to_string(),
                validator_version: "validator".to_string(),
            },
        );
        assert!(
            failure
                .errors
                .iter()
                .any(|error| error.message.contains("more than one direct outgoing edge"))
        );

        let mut registry = WorkflowRegistry::new();
        let failure = register_workflow_err(
            &mut registry,
            &capabilities,
            WorkflowRegistration {
                scope: RegistryScope::Public,
                definition: WorkflowDefinition {
                    nodes: Vec::new(),
                    terminal_nodes: vec!["persist_comment".to_string()],
                    ..valid_workflow_definition()
                },
                workflow_path: "workflows/no-nodes.json".to_string(),
                registered_at: "2026-03-27T00:00:00Z".to_string(),
                validator_version: "validator".to_string(),
            },
        );
        assert!(failure.errors.iter().any(|error| error.path == "$.nodes"));
    }

    #[test]
    fn helper_paths_cover_workflow_artifact_and_lookup_order() {
        let artifact = workflow_artifact_record("wf", "1.0.0", "artifact");
        assert_eq!(artifact.implementation_kind, ImplementationKind::Workflow);
        assert!(artifact.binary.is_none());
        assert_eq!(
            artifact.workflow_ref,
            Some(WorkflowReference {
                workflow_id: "wf".to_string(),
                workflow_version: "1.0.0".to_string(),
            })
        );
        assert_eq!(
            lookup_order(LookupScope::PublicOnly),
            &[RegistryScope::Public]
        );
        assert_eq!(
            lookup_order(LookupScope::PreferPrivate),
            &[RegistryScope::Private, RegistryScope::Public]
        );
    }

    // ── schema_property_types and helper coverage ─────────────────────────────

    #[test]
    fn schema_property_types_covers_non_object_and_malformed_schemas() {
        // Line 828: schema is not a JSON object value at all.
        assert!(schema_property_types(&json!("not-an-object")).is_empty());
        // Line 831: schema is an object but has no "type" key.
        assert!(schema_property_types(&json!({"properties": {}})).is_empty());
        // Line 834: has "type" key but it is not "object".
        assert!(schema_property_types(&json!({"type": "array"})).is_empty());
        // Line 837: type is "object" but no "properties" key.
        assert!(schema_property_types(&json!({"type": "object"})).is_empty());

        // Lines 843 + 846: properties map has entries that are not JSON objects
        // (line 843) and entries that are objects without a "type" key (line 846).
        let types = schema_property_types(&json!({
            "type": "object",
            "properties": {
                "valid":       { "type": "string" },
                "non_object":  "just-a-string",
                "no_type_key": { "description": "no type field here" }
            }
        }));
        assert_eq!(types.len(), 1);
        assert_eq!(types["valid"], "string");
    }

    #[test]
    fn schema_declares_properties_returns_false_for_non_object_schema() {
        assert!(!schema_declares_properties(&json!("string-schema")));
        assert!(!schema_declares_properties(&json!(null)));
        assert!(!schema_declares_properties(&json!(42)));
        assert!(!schema_declares_properties(&json!({"type": "array"})));
        assert!(!schema_declares_properties(&json!({"type": "object"})));
        assert!(schema_declares_properties(
            &json!({"type": "object", "properties": {}})
        ));
    }

    // ── validate_schema_compatibility coverage ────────────────────────────────

    #[allow(clippy::needless_pass_by_value)]
    fn capability_registry_with_override(
        id: &str,
        inputs: Option<Value>,
        outputs: Option<Value>,
    ) -> CapabilityRegistry {
        let mut registry = CapabilityRegistry::new();
        for registration in [
            capability_registration(
                "content.comments.create-comment-draft",
                vec![EventReference {
                    event_id: "content.comments.draft-created".to_string(),
                    version: "1.0.0".to_string(),
                }],
            ),
            capability_registration(
                "content.comments.validate-comment",
                vec![EventReference {
                    event_id: "content.comments.validated".to_string(),
                    version: "1.0.0".to_string(),
                }],
            ),
            capability_registration("content.comments.persist-comment", Vec::new()),
        ] {
            let mut reg = registration;
            if reg.contract.id == id {
                if let Some(s) = inputs.clone() {
                    reg.contract.inputs.schema = s;
                }
                if let Some(s) = outputs.clone() {
                    reg.contract.outputs.schema = s;
                }
            }
            registry
                .register(reg)
                .expect("capability registration should succeed");
        }
        registry
    }

    #[test]
    fn validates_schema_compatibility_with_open_capability_input_schema() {
        // Covers lines 742 (BTreeMap::new()) and 759-761 (!cap_input_has_properties → continue).
        // When the capability's input schema is open (no `properties` key), field-level
        // type-checking is skipped; the workflow must still succeed.
        let caps = capability_registry_with_override(
            "content.comments.create-comment-draft",
            Some(json!({"type": "object"})),
            None,
        );
        let mut registry = WorkflowRegistry::new();
        register_workflow_ok(
            &mut registry,
            &caps,
            WorkflowRegistration {
                scope: RegistryScope::Public,
                definition: valid_workflow_definition(),
                workflow_path: "workflows/open-cap-input.json".to_string(),
                registered_at: "2026-03-27T00:00:00Z".to_string(),
                validator_version: "workflow-validator/0.1.0".to_string(),
            },
        );
    }

    #[test]
    fn rejects_when_capability_input_schema_missing_referenced_field() {
        // Covers lines 763-771: state contains a field that the capability's closed
        // input schema does not declare → EdgeSchemaMismatch.
        let caps = capability_registry_with_override(
            "content.comments.create-comment-draft",
            Some(json!({
                "type": "object",
                "properties": {
                    "comment_text": { "type": "string" }
                    // intentionally omits "extra_field"
                }
            })),
            None,
        );

        let mut definition = valid_workflow_definition();
        // Add extra_field to workflow input schema (so it is in state).
        definition.inputs.schema = json!({
            "type": "object",
            "properties": {
                "comment_text": { "type": "string" },
                "extra_field":  { "type": "string" }
            },
            "required": ["comment_text"],
            "additionalProperties": true
        });
        // Make the first node also pull extra_field — it is in state but not in cap schema.
        definition.nodes[0]
            .input
            .from_workflow_input
            .push("extra_field".to_string());

        let mut registry = WorkflowRegistry::new();
        let failure = register_workflow_err(
            &mut registry,
            &caps,
            WorkflowRegistration {
                scope: RegistryScope::Public,
                definition,
                workflow_path: "workflows/missing-cap-input-field.json".to_string(),
                registered_at: "2026-03-27T00:00:00Z".to_string(),
                validator_version: "validator".to_string(),
            },
        );
        assert!(failure.errors.iter().any(|e| {
            e.code == WorkflowErrorCode::EdgeSchemaMismatch
                && e.message
                    .contains("capability input schema is missing referenced field")
        }));
    }

    #[test]
    fn rejects_field_type_mismatch_between_state_and_capability_input() {
        // Covers lines 778-789: state supplies a field as `integer` but the capability
        // input schema declares it as `string` → EdgeSchemaMismatch with type mismatch.
        let caps = capability_registry(); // default: draft_id: string in cap input
        let mut definition = valid_workflow_definition();
        // Override workflow input so draft_id enters state as integer.
        definition.inputs.schema = json!({
            "type": "object",
            "properties": {
                "comment_text": { "type": "string" },
                "draft_id":     { "type": "integer" }
            },
            "required": ["comment_text"],
            "additionalProperties": true
        });
        // First node takes draft_id directly from workflow input.
        definition.nodes[0]
            .input
            .from_workflow_input
            .push("draft_id".to_string());

        let mut registry = WorkflowRegistry::new();
        let failure = register_workflow_err(
            &mut registry,
            &caps,
            WorkflowRegistration {
                scope: RegistryScope::Public,
                definition,
                workflow_path: "workflows/type-mismatch.json".to_string(),
                registered_at: "2026-03-27T00:00:00Z".to_string(),
                validator_version: "validator".to_string(),
            },
        );
        assert!(failure.errors.iter().any(|e| {
            e.code == WorkflowErrorCode::EdgeSchemaMismatch
                && e.message.contains("field type mismatch")
        }));
    }

    #[test]
    fn rejects_when_capability_output_schema_missing_referenced_field() {
        // Covers lines 796-804: to_workflow_state references a field not present in
        // the capability's closed output schema → EdgeSchemaMismatch.
        let caps = capability_registry_with_override(
            "content.comments.create-comment-draft",
            None,
            Some(json!({
                "type": "object",
                "properties": {
                    "draft_id": { "type": "string" }
                    // intentionally omits "result_token"
                }
            })),
        );

        let mut definition = valid_workflow_definition();
        // First node asks to write result_token into state, but cap doesn't output it.
        definition.nodes[0]
            .output
            .to_workflow_state
            .push("result_token".to_string());

        let mut registry = WorkflowRegistry::new();
        let failure = register_workflow_err(
            &mut registry,
            &caps,
            WorkflowRegistration {
                scope: RegistryScope::Public,
                definition,
                workflow_path: "workflows/missing-cap-output-field.json".to_string(),
                registered_at: "2026-03-27T00:00:00Z".to_string(),
                validator_version: "validator".to_string(),
            },
        );
        assert!(failure.errors.iter().any(|e| {
            e.code == WorkflowErrorCode::EdgeSchemaMismatch
                && e.message
                    .contains("capability output schema is missing referenced field")
        }));
    }

    #[test]
    fn accepts_open_capability_output_schema_and_infers_object_type_for_state() {
        // Covers lines 808-812: open capability output schema → to_workflow_state keys are
        // added to state with inferred type "object" rather than rejected.
        // create-comment-draft has open output; validate-comment has open input so that
        // the inferred "object" type for draft_id does not cause a type-mismatch on the
        // next node.  validate-comment's closed output then restores draft_id as "string"
        // so persist-comment's closed input schema is satisfied.
        let mut caps = CapabilityRegistry::new();

        let mut create_reg = capability_registration(
            "content.comments.create-comment-draft",
            vec![EventReference {
                event_id: "content.comments.draft-created".to_string(),
                version: "1.0.0".to_string(),
            }],
        );
        // Open output schema — forces the `else` branch in validate_schema_compatibility.
        create_reg.contract.outputs.schema = json!({"type": "object"});
        caps.register(create_reg)
            .expect("capability registration should succeed");

        let mut validate_reg = capability_registration(
            "content.comments.validate-comment",
            vec![EventReference {
                event_id: "content.comments.validated".to_string(),
                version: "1.0.0".to_string(),
            }],
        );
        // Open input schema — skips type-checking on the "object"-typed draft_id from state.
        validate_reg.contract.inputs.schema = json!({"type": "object"});
        caps.register(validate_reg)
            .expect("capability registration should succeed");

        caps.register(capability_registration(
            "content.comments.persist-comment",
            Vec::new(),
        ))
        .expect("capability registration should succeed");

        let mut registry = WorkflowRegistry::new();
        register_workflow_ok(
            &mut registry,
            &caps,
            WorkflowRegistration {
                scope: RegistryScope::Public,
                definition: valid_workflow_definition(),
                workflow_path: "workflows/open-cap-output.json".to_string(),
                registered_at: "2026-03-27T00:00:00Z".to_string(),
                validator_version: "workflow-validator/0.1.0".to_string(),
            },
        );
    }

    fn capability_registry() -> CapabilityRegistry {
        let mut registry = CapabilityRegistry::new();
        for registration in [
            capability_registration(
                "content.comments.create-comment-draft",
                vec![EventReference {
                    event_id: "content.comments.draft-created".to_string(),
                    version: "1.0.0".to_string(),
                }],
            ),
            capability_registration(
                "content.comments.validate-comment",
                vec![EventReference {
                    event_id: "content.comments.validated".to_string(),
                    version: "1.0.0".to_string(),
                }],
            ),
            capability_registration("content.comments.persist-comment", Vec::new()),
        ] {
            register_capability_ok(&mut registry, registration);
        }
        registry
    }

    #[allow(clippy::too_many_lines)]
    fn capability_registration(
        capability_id: &str,
        emits: Vec<EventReference>,
    ) -> CapabilityRegistration {
        CapabilityRegistration {
            scope: RegistryScope::Public,
            contract: traverse_contracts::CapabilityContract {
                kind: "capability_contract".to_string(),
                schema_version: "1.0.0".to_string(),
                id: capability_id.to_string(),
                namespace: "content.comments".to_string(),
                name: capability_id
                    .rsplit('.')
                    .next()
                    .unwrap_or("capability")
                    .to_string(),
                version: "1.0.0".to_string(),
                lifecycle: Lifecycle::Active,
                owner: Owner {
                    team: "comments".to_string(),
                    contact: "comments@example.com".to_string(),
                },
                summary: "fixture capability for workflow tests".to_string(),
                description: "fixture capability used by workflow registry tests".to_string(),
                inputs: SchemaContainer {
                    schema: json!({
                        "type": "object",
                        "properties": {
                            "comment_text": { "type": "string" },
                            "draft_id": { "type": "string" }
                        },
                        "additionalProperties": true
                    }),
                },
                outputs: SchemaContainer {
                    schema: json!({
                        "type": "object",
                        "properties": {
                            "draft_id": { "type": "string" },
                            "comment_id": { "type": "string" }
                        },
                        "additionalProperties": true
                    }),
                },
                preconditions: vec![Condition {
                    id: "precondition".to_string(),
                    description: "must be valid".to_string(),
                }],
                postconditions: vec![Condition {
                    id: "postcondition".to_string(),
                    description: "must produce output".to_string(),
                }],
                side_effects: vec![SideEffect {
                    kind: SideEffectKind::MemoryOnly,
                    description: "memory only".to_string(),
                }],
                emits,
                consumes: Vec::new(),
                permissions: Vec::new(),
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
                policies: Vec::new(),
                dependencies: Vec::new(),
                provenance: Provenance {
                    source: ProvenanceSource::Greenfield,
                    author: "Enrico".to_string(),
                    created_at: "2026-03-27T00:00:00Z".to_string(),
                    spec_ref: Some("005-capability-registry".to_string()),
                    adr_refs: Vec::new(),
                    exception_refs: Vec::new(),
                },
                evidence: vec![ValidationEvidence {
                    evidence_id: "evidence-fixture".to_string(),
                    evidence_type: EvidenceType::ContractValidation,
                    status: EvidenceStatus::Passed,
                }],
                service_type: ServiceType::Stateless,
                permitted_targets: vec![
                    ExecutionTarget::Local,
                    ExecutionTarget::Cloud,
                    ExecutionTarget::Edge,
                    ExecutionTarget::Device,
                ],
                event_trigger: None,
                connector_requirements: Vec::new(),
                state_schema: None,
            },
            contract_path: format!("contracts/{capability_id}.json"),
            artifact: CapabilityArtifactRecord {
                artifact_ref: format!("artifact-{capability_id}"),
                implementation_kind: ImplementationKind::Executable,
                source: SourceReference {
                    kind: SourceKind::Git,
                    location: format!("https://example.com/{capability_id}.git"),
                },
                binary: Some(BinaryReference {
                    format: BinaryFormat::Wasm,
                    location: format!("{capability_id}.wasm"),
                }),
                workflow_ref: None,
                digests: ArtifactDigests {
                    source_digest: "source".to_string(),
                    binary_digest: Some("binary".to_string()),
                },
                provenance: RegistryProvenance {
                    source: "fixtures".to_string(),
                    author: "Enrico".to_string(),
                    created_at: "2026-03-27T00:00:00Z".to_string(),
                },
            },
            registered_at: "2026-03-27T00:00:00Z".to_string(),
            tags: vec!["comments".to_string()],
            composability: ComposabilityMetadata {
                kind: CompositionKind::Atomic,
                patterns: vec![CompositionPattern::Sequential],
                provides: vec!["comment".to_string()],
                requires: Vec::new(),
            },
            governing_spec: "005-capability-registry".to_string(),
            validator_version: "validator".to_string(),
        }
    }

    fn valid_workflow_definition() -> WorkflowDefinition {
        WorkflowDefinition {
            kind: WORKFLOW_KIND.to_string(),
            schema_version: WORKFLOW_SCHEMA_VERSION.to_string(),
            id: "content.comments.publish-comment".to_string(),
            name: "publish-comment".to_string(),
            version: "1.0.0".to_string(),
            lifecycle: Lifecycle::Active,
            owner: Owner {
                team: "comments".to_string(),
                contact: "comments@example.com".to_string(),
            },
            summary: "Publish a comment deterministically.".to_string(),
            inputs: SchemaContainer {
                schema: json!({
                    "type": "object",
                    "properties": {
                        "comment_text": { "type": "string" }
                    },
                    "required": ["comment_text"],
                    "additionalProperties": true
                }),
            },
            outputs: SchemaContainer {
                schema: json!({
                    "type": "object",
                    "properties": {
                        "comment_id": { "type": "string" }
                    },
                    "required": ["comment_id"],
                    "additionalProperties": true
                }),
            },
            nodes: vec![
                WorkflowNode {
                    node_id: "create_draft".to_string(),
                    capability_id: "content.comments.create-comment-draft".to_string(),
                    capability_version: "1.0.0".to_string(),
                    input: WorkflowNodeInput {
                        from_workflow_input: vec!["comment_text".to_string()],
                    },
                    output: WorkflowNodeOutput {
                        to_workflow_state: vec!["draft_id".to_string()],
                    },
                },
                WorkflowNode {
                    node_id: "validate_comment".to_string(),
                    capability_id: "content.comments.validate-comment".to_string(),
                    capability_version: "1.0.0".to_string(),
                    input: WorkflowNodeInput {
                        from_workflow_input: vec!["draft_id".to_string()],
                    },
                    output: WorkflowNodeOutput {
                        to_workflow_state: vec!["draft_id".to_string()],
                    },
                },
                WorkflowNode {
                    node_id: "persist_comment".to_string(),
                    capability_id: "content.comments.persist-comment".to_string(),
                    capability_version: "1.0.0".to_string(),
                    input: WorkflowNodeInput {
                        from_workflow_input: vec!["draft_id".to_string()],
                    },
                    output: WorkflowNodeOutput {
                        to_workflow_state: vec!["comment_id".to_string()],
                    },
                },
            ],
            edges: vec![
                WorkflowEdge {
                    edge_id: "draft_to_validate".to_string(),
                    from: "create_draft".to_string(),
                    to: "validate_comment".to_string(),
                    trigger: WorkflowEdgeTrigger::Event,
                    event: Some(EventReference {
                        event_id: "content.comments.draft-created".to_string(),
                        version: "1.0.0".to_string(),
                    }),
                    predicate: None,
                },
                WorkflowEdge {
                    edge_id: "validate_to_persist".to_string(),
                    from: "validate_comment".to_string(),
                    to: "persist_comment".to_string(),
                    trigger: WorkflowEdgeTrigger::Event,
                    event: Some(EventReference {
                        event_id: "content.comments.validated".to_string(),
                        version: "1.0.0".to_string(),
                    }),
                    predicate: None,
                },
            ],
            start_node: "create_draft".to_string(),
            terminal_nodes: vec!["persist_comment".to_string()],
            tags: vec!["comments".to_string(), "foundation".to_string()],
            governing_spec: WORKFLOW_GOVERNING_SPEC.to_string(),
        }
    }

    fn register_workflow_ok(
        registry: &mut WorkflowRegistry,
        capabilities: &CapabilityRegistry,
        request: WorkflowRegistration,
    ) -> WorkflowRegistrationOutcome {
        registry
            .register(capabilities, request)
            .expect("workflow registration should succeed")
    }

    fn register_workflow_err(
        registry: &mut WorkflowRegistry,
        capabilities: &CapabilityRegistry,
        request: WorkflowRegistration,
    ) -> WorkflowFailure {
        registry
            .register(capabilities, request)
            .expect_err("workflow registration should fail")
    }

    fn find_workflow_exact(
        registry: &WorkflowRegistry,
        scope: LookupScope,
        id: &str,
        version: &str,
    ) -> ResolvedWorkflow {
        registry
            .find_exact(scope, id, version)
            .expect("workflow should be present")
    }

    fn register_capability_ok(registry: &mut CapabilityRegistry, request: CapabilityRegistration) {
        registry
            .register(request)
            .expect("capability registration should succeed");
    }
}
