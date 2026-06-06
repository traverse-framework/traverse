use crate::{
    CapabilityRegistry, EventRegistry, LookupScope, RegistryScope, WorkflowRegistry,
    compare_versions,
};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use traverse_contracts::Lifecycle;

const METADATA_GRAPH_KIND: &str = "metadata_graph_snapshot";
const METADATA_GRAPH_SCHEMA_VERSION: &str = "1.0.0";
const METADATA_GRAPH_GOVERNING_SPEC: &str = "015-metadata-graph";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataGraphSnapshot {
    pub kind: String,
    pub schema_version: String,
    pub governing_spec: String,
    pub generated_at: String,
    pub evidence: MetadataGraphGenerationEvidence,
    pub nodes: Vec<MetadataGraphNode>,
    pub edges: Vec<MetadataGraphEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataGraphGenerationEvidence {
    pub source_specs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MetadataGraphNodeKind {
    Capability,
    Event,
    Workflow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataGraphNode {
    pub node_id: String,
    pub kind: MetadataGraphNodeKind,
    pub scope: RegistryScope,
    pub artifact_id: String,
    pub version: String,
    pub lifecycle: Lifecycle,
    pub summary: String,
    pub owner_team: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MetadataGraphEdgeKind {
    References,
    Publishes,
    SubscribesTo,
    Composes,
    Supersedes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataGraphEdge {
    pub edge_id: String,
    pub kind: MetadataGraphEdgeKind,
    pub from_node_id: String,
    pub to_node_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataGraphLookupScope {
    All,
    PublicOnly,
    PreferPrivate,
}

#[allow(clippy::too_many_lines)]
#[must_use]
pub fn project_metadata_graph(
    capabilities: &CapabilityRegistry,
    events: &EventRegistry,
    workflows: &WorkflowRegistry,
    generated_at: &str,
) -> MetadataGraphSnapshot {
    let capability_entries = capabilities.graph_entries();
    let event_entries = events.graph_entries();
    let workflow_entries = workflows.graph_entries();

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut seen_node_ids = BTreeSet::new();

    for capability in &capability_entries {
        let node = MetadataGraphNode {
            node_id: capability_node_id(
                capability.record.scope,
                &capability.record.id,
                &capability.record.version,
            ),
            kind: MetadataGraphNodeKind::Capability,
            scope: capability.record.scope,
            artifact_id: capability.record.id.clone(),
            version: capability.record.version.clone(),
            lifecycle: capability.record.lifecycle.clone(),
            summary: capability.index_entry.summary.clone(),
            owner_team: capability.record.owner.team.clone(),
        };
        if seen_node_ids.insert(node.node_id.clone()) {
            nodes.push(node);
        }
    }

    for event in &event_entries {
        let node = MetadataGraphNode {
            node_id: event_node_id(event.record.scope, &event.record.id, &event.record.version),
            kind: MetadataGraphNodeKind::Event,
            scope: event.record.scope,
            artifact_id: event.record.id.clone(),
            version: event.record.version.clone(),
            lifecycle: event.record.lifecycle.clone(),
            summary: event.record.summary.clone(),
            owner_team: event.record.owner.team.clone(),
        };
        if seen_node_ids.insert(node.node_id.clone()) {
            nodes.push(node);
        }
    }

    for workflow in &workflow_entries {
        let node = MetadataGraphNode {
            node_id: workflow_node_id(
                workflow.record.scope,
                &workflow.record.id,
                &workflow.record.version,
            ),
            kind: MetadataGraphNodeKind::Workflow,
            scope: workflow.record.scope,
            artifact_id: workflow.record.id.clone(),
            version: workflow.record.version.clone(),
            lifecycle: workflow.record.lifecycle.clone(),
            summary: workflow.index_entry.summary.clone(),
            owner_team: workflow.record.owner.team.clone(),
        };
        if seen_node_ids.insert(node.node_id.clone()) {
            nodes.push(node);
        }
    }

    for capability in &capability_entries {
        let from_node_id = capability_node_id(
            capability.record.scope,
            &capability.record.id,
            &capability.record.version,
        );
        for event_ref in &capability.contract.emits {
            for scope in [RegistryScope::Private, RegistryScope::Public] {
                if events
                    .find_exact(
                        LookupScope::PreferPrivate,
                        &event_ref.event_id,
                        &event_ref.version,
                    )
                    .is_some()
                    && event_exists(
                        &event_entries,
                        scope,
                        &event_ref.event_id,
                        &event_ref.version,
                    )
                {
                    let to_node_id = event_node_id(scope, &event_ref.event_id, &event_ref.version);
                    edges.push(build_edge(
                        MetadataGraphEdgeKind::Publishes,
                        &from_node_id,
                        &to_node_id,
                    ));
                }
            }
        }
        for event_ref in &capability.contract.consumes {
            for scope in [RegistryScope::Private, RegistryScope::Public] {
                if events
                    .find_exact(
                        LookupScope::PreferPrivate,
                        &event_ref.event_id,
                        &event_ref.version,
                    )
                    .is_some()
                    && event_exists(
                        &event_entries,
                        scope,
                        &event_ref.event_id,
                        &event_ref.version,
                    )
                {
                    let to_node_id = event_node_id(scope, &event_ref.event_id, &event_ref.version);
                    edges.push(build_edge(
                        MetadataGraphEdgeKind::SubscribesTo,
                        &from_node_id,
                        &to_node_id,
                    ));
                }
            }
        }
        if let Some(workflow_ref) = capability.artifact.workflow_ref.as_ref() {
            for scope in [RegistryScope::Private, RegistryScope::Public] {
                if workflow_exists(
                    &workflow_entries,
                    scope,
                    &workflow_ref.workflow_id,
                    &workflow_ref.workflow_version,
                ) {
                    edges.push(build_edge(
                        MetadataGraphEdgeKind::References,
                        &from_node_id,
                        &workflow_node_id(
                            scope,
                            &workflow_ref.workflow_id,
                            &workflow_ref.workflow_version,
                        ),
                    ));
                }
            }
        }
    }

    for workflow in &workflow_entries {
        let from_node_id = workflow_node_id(
            workflow.record.scope,
            &workflow.record.id,
            &workflow.record.version,
        );
        for node in &workflow.definition.nodes {
            for scope in [RegistryScope::Private, RegistryScope::Public] {
                if capability_exists(
                    &capability_entries,
                    scope,
                    &node.capability_id,
                    &node.capability_version,
                ) {
                    edges.push(build_edge(
                        MetadataGraphEdgeKind::Composes,
                        &from_node_id,
                        &capability_node_id(scope, &node.capability_id, &node.capability_version),
                    ));
                }
            }
        }
        for edge in &workflow.definition.edges {
            if let Some(event_ref) = edge.event.as_ref() {
                for scope in [RegistryScope::Private, RegistryScope::Public] {
                    if event_exists(
                        &event_entries,
                        scope,
                        &event_ref.event_id,
                        &event_ref.version,
                    ) {
                        edges.push(build_edge(
                            MetadataGraphEdgeKind::References,
                            &from_node_id,
                            &event_node_id(scope, &event_ref.event_id, &event_ref.version),
                        ));
                    }
                }
            }
        }
    }

    edges.extend(version_lineage_edges(
        &nodes,
        MetadataGraphNodeKind::Capability,
        capability_node_id,
    ));
    edges.extend(version_lineage_edges(
        &nodes,
        MetadataGraphNodeKind::Event,
        event_node_id,
    ));
    edges.extend(version_lineage_edges(
        &nodes,
        MetadataGraphNodeKind::Workflow,
        workflow_node_id,
    ));

    nodes.sort_by(compare_nodes);
    edges.sort_by(compare_edges);
    edges.dedup_by(|left, right| left.edge_id == right.edge_id);

    MetadataGraphSnapshot {
        kind: METADATA_GRAPH_KIND.to_string(),
        schema_version: METADATA_GRAPH_SCHEMA_VERSION.to_string(),
        governing_spec: METADATA_GRAPH_GOVERNING_SPEC.to_string(),
        generated_at: generated_at.to_string(),
        evidence: MetadataGraphGenerationEvidence {
            source_specs: vec![
                "005-capability-registry".to_string(),
                "007-workflow-registry-traversal".to_string(),
                "011-event-registry".to_string(),
                METADATA_GRAPH_GOVERNING_SPEC.to_string(),
            ],
        },
        nodes,
        edges,
    }
}

impl MetadataGraphSnapshot {
    #[must_use]
    pub fn find_node(
        &self,
        lookup_scope: MetadataGraphLookupScope,
        kind: MetadataGraphNodeKind,
        artifact_id: &str,
        version: &str,
    ) -> Option<&MetadataGraphNode> {
        let mut matching = self
            .nodes
            .iter()
            .filter(|node| {
                node.kind == kind && node.artifact_id == artifact_id && node.version == version
            })
            .collect::<Vec<_>>();

        matching.sort_by(|left, right| {
            compare_scopes(left.scope, right.scope).then_with(|| compare_nodes(left, right))
        });

        match lookup_scope {
            MetadataGraphLookupScope::All => matching.into_iter().next(),
            MetadataGraphLookupScope::PublicOnly => matching
                .into_iter()
                .find(|node| node.scope == RegistryScope::Public),
            MetadataGraphLookupScope::PreferPrivate => matching.into_iter().find(|node| {
                node.scope == RegistryScope::Private || node.scope == RegistryScope::Public
            }),
        }
    }

    #[must_use]
    pub fn outgoing_edges(&self, from_node_id: &str) -> Vec<&MetadataGraphEdge> {
        self.edges
            .iter()
            .filter(|edge| edge.from_node_id == from_node_id)
            .collect()
    }
}

fn capability_node_id(scope: RegistryScope, id: &str, version: &str) -> String {
    format!("capability:{}:{id}:{version}", scope_name(scope))
}

fn event_node_id(scope: RegistryScope, id: &str, version: &str) -> String {
    format!("event:{}:{id}:{version}", scope_name(scope))
}

fn workflow_node_id(scope: RegistryScope, id: &str, version: &str) -> String {
    format!("workflow:{}:{id}:{version}", scope_name(scope))
}

fn build_edge(
    kind: MetadataGraphEdgeKind,
    from_node_id: &str,
    to_node_id: &str,
) -> MetadataGraphEdge {
    MetadataGraphEdge {
        edge_id: format!("{}:{}:{}", edge_kind_name(kind), from_node_id, to_node_id),
        kind,
        from_node_id: from_node_id.to_string(),
        to_node_id: to_node_id.to_string(),
    }
}

fn version_lineage_edges<F>(
    nodes: &[MetadataGraphNode],
    kind: MetadataGraphNodeKind,
    node_id: F,
) -> Vec<MetadataGraphEdge>
where
    F: Fn(RegistryScope, &str, &str) -> String,
{
    let mut grouped = BTreeMap::<(RegistryScope, String), Vec<&MetadataGraphNode>>::new();
    for node in nodes.iter().filter(|node| node.kind == kind) {
        grouped
            .entry((node.scope, node.artifact_id.clone()))
            .or_default()
            .push(node);
    }

    let mut edges = Vec::new();
    for ((scope, artifact_id), mut versions) in grouped {
        versions.sort_by(|left, right| compare_versions(&left.version, &right.version));
        for pair in versions.windows(2) {
            if let [previous, next] = pair {
                edges.push(build_edge(
                    MetadataGraphEdgeKind::Supersedes,
                    &node_id(scope, &artifact_id, &next.version),
                    &node_id(scope, &artifact_id, &previous.version),
                ));
            }
        }
    }
    edges
}

fn capability_exists(
    entries: &[crate::ResolvedCapability],
    scope: RegistryScope,
    id: &str,
    version: &str,
) -> bool {
    entries.iter().any(|entry| {
        entry.record.scope == scope && entry.record.id == id && entry.record.version == version
    })
}

fn event_exists(
    entries: &[crate::ResolvedEvent],
    scope: RegistryScope,
    id: &str,
    version: &str,
) -> bool {
    entries.iter().any(|entry| {
        entry.record.scope == scope && entry.record.id == id && entry.record.version == version
    })
}

fn workflow_exists(
    entries: &[crate::ResolvedWorkflow],
    scope: RegistryScope,
    id: &str,
    version: &str,
) -> bool {
    entries.iter().any(|entry| {
        entry.record.scope == scope && entry.record.id == id && entry.record.version == version
    })
}

fn compare_nodes(left: &MetadataGraphNode, right: &MetadataGraphNode) -> Ordering {
    left.kind
        .cmp(&right.kind)
        .then_with(|| compare_scopes(left.scope, right.scope))
        .then_with(|| left.artifact_id.cmp(&right.artifact_id))
        .then_with(|| compare_versions(&right.version, &left.version))
}

fn compare_edges(left: &MetadataGraphEdge, right: &MetadataGraphEdge) -> Ordering {
    left.kind
        .cmp(&right.kind)
        .then_with(|| left.from_node_id.cmp(&right.from_node_id))
        .then_with(|| left.to_node_id.cmp(&right.to_node_id))
}

fn compare_scopes(left: RegistryScope, right: RegistryScope) -> Ordering {
    match (left, right) {
        (RegistryScope::Private, RegistryScope::Public) => Ordering::Less,
        (RegistryScope::Public, RegistryScope::Private) => Ordering::Greater,
        _ => Ordering::Equal,
    }
}

fn scope_name(scope: RegistryScope) -> &'static str {
    match scope {
        RegistryScope::Public => "public",
        RegistryScope::Private => "private",
    }
}

fn edge_kind_name(kind: MetadataGraphEdgeKind) -> &'static str {
    match kind {
        MetadataGraphEdgeKind::References => "references",
        MetadataGraphEdgeKind::Publishes => "publishes",
        MetadataGraphEdgeKind::SubscribesTo => "subscribes_to",
        MetadataGraphEdgeKind::Composes => "composes",
        MetadataGraphEdgeKind::Supersedes => "supersedes",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MetadataGraphEdgeKind, MetadataGraphLookupScope, MetadataGraphNodeKind,
        project_metadata_graph,
    };
    use crate::{
        ArtifactDigests, BinaryFormat, BinaryReference, CapabilityArtifactRecord,
        CapabilityRegistration, CapabilityRegistry, ComposabilityMetadata, CompositionKind,
        CompositionPattern, EventRegistration, EventRegistry, ImplementationKind,
        RegistryProvenance, RegistryScope, SourceKind, SourceReference, WorkflowDefinition,
        WorkflowEdge, WorkflowEdgeTrigger, WorkflowNode, WorkflowNodeInput, WorkflowNodeOutput,
        WorkflowRegistration, WorkflowRegistry,
    };
    use serde_json::json;
    use traverse_contracts::{
        BinaryFormat as ContractBinaryFormat, CapabilityContract, CapabilityReference, Condition,
        Entrypoint, EntrypointKind, EventClassification, EventContract, EventPayload,
        EventProvenance, EventProvenanceSource, EventReference, EventType, Execution,
        ExecutionConstraints, ExecutionTarget, FilesystemAccess, HostApiAccess, Lifecycle,
        NetworkAccess, Owner, PayloadCompatibility, Provenance, ProvenanceSource, SchemaContainer,
        ServiceType, SideEffect, SideEffectKind,
    };

    #[test]
    fn projects_deterministic_nodes_and_edges() {
        let mut capabilities = CapabilityRegistry::new();
        let mut events = EventRegistry::new();
        let mut workflows = WorkflowRegistry::new();

        let create = capability_registration(
            RegistryScope::Public,
            capability_contract(
                "content.comments.create-comment-draft",
                "1.0.0",
                vec![],
                vec![EventReference {
                    event_id: "content.comments.comment-draft-created".to_string(),
                    version: "1.0.0".to_string(),
                }],
            ),
            ImplementationKind::Executable,
            None,
        );
        let publish = capability_registration(
            RegistryScope::Private,
            capability_contract(
                "content.comments.publish-comment",
                "1.0.0",
                vec![EventReference {
                    event_id: "content.comments.comment-draft-created".to_string(),
                    version: "1.0.0".to_string(),
                }],
                vec![],
            ),
            ImplementationKind::Workflow,
            Some(("content.comments.publish-comment-flow", "1.0.0")),
        );
        assert!(capabilities.register(create).is_ok());
        assert!(capabilities.register(publish).is_ok());
        assert!(
            events
                .register(event_registration(
                    RegistryScope::Public,
                    event_contract("content.comments.comment-draft-created", "1.0.0"),
                ))
                .is_ok()
        );
        assert!(
            workflows
                .register(
                    &capabilities,
                    workflow_registration(
                        RegistryScope::Private,
                        workflow_definition("content.comments.publish-comment-flow", "1.0.0"),
                    ),
                )
                .is_ok()
        );

        let snapshot =
            project_metadata_graph(&capabilities, &events, &workflows, "2026-03-30T00:00:00Z");

        assert_eq!(snapshot.kind, "metadata_graph_snapshot");
        assert_eq!(snapshot.nodes.len(), 4);
        assert!(snapshot.edges.iter().any(|edge| {
            edge.kind == MetadataGraphEdgeKind::Publishes
                && edge.from_node_id
                    == "capability:public:content.comments.create-comment-draft:1.0.0"
                && edge.to_node_id == "event:public:content.comments.comment-draft-created:1.0.0"
        }));
        assert!(snapshot.edges.iter().any(|edge| {
            edge.kind == MetadataGraphEdgeKind::SubscribesTo
                && edge.from_node_id == "capability:private:content.comments.publish-comment:1.0.0"
                && edge.to_node_id == "event:public:content.comments.comment-draft-created:1.0.0"
        }));
        assert!(snapshot.edges.iter().any(|edge| {
            edge.kind == MetadataGraphEdgeKind::Composes
                && edge.from_node_id
                    == "workflow:private:content.comments.publish-comment-flow:1.0.0"
        }));
        assert!(snapshot.edges.iter().any(|edge| {
            edge.kind == MetadataGraphEdgeKind::References
                && edge.from_node_id == "capability:private:content.comments.publish-comment:1.0.0"
                && edge.to_node_id == "workflow:private:content.comments.publish-comment-flow:1.0.0"
        }));
    }

    #[test]
    fn graph_lookup_prefers_private_and_emits_version_lineage() {
        let mut capabilities = CapabilityRegistry::new();
        let events = EventRegistry::new();
        let workflows = WorkflowRegistry::new();

        assert!(
            capabilities
                .register(capability_registration(
                    RegistryScope::Public,
                    capability_contract(
                        "content.comments.create-comment-draft",
                        "1.0.0",
                        vec![],
                        vec![]
                    ),
                    ImplementationKind::Executable,
                    None,
                ))
                .is_ok()
        );
        assert!(
            capabilities
                .register(capability_registration(
                    RegistryScope::Public,
                    capability_contract(
                        "content.comments.create-comment-draft",
                        "1.1.0",
                        vec![],
                        vec![]
                    ),
                    ImplementationKind::Executable,
                    None,
                ))
                .is_ok()
        );
        assert!(
            capabilities
                .register(capability_registration(
                    RegistryScope::Private,
                    capability_contract(
                        "content.comments.create-comment-draft",
                        "1.1.0",
                        vec![],
                        vec![]
                    ),
                    ImplementationKind::Executable,
                    None,
                ))
                .is_ok()
        );

        let snapshot =
            project_metadata_graph(&capabilities, &events, &workflows, "2026-03-30T00:00:00Z");

        let resolved = snapshot.find_node(
            MetadataGraphLookupScope::PreferPrivate,
            MetadataGraphNodeKind::Capability,
            "content.comments.create-comment-draft",
            "1.1.0",
        );
        assert_eq!(
            resolved.map(|node| node.scope),
            Some(RegistryScope::Private)
        );

        let supersedes = snapshot
            .edges
            .iter()
            .filter(|edge| edge.kind == MetadataGraphEdgeKind::Supersedes)
            .collect::<Vec<_>>();
        assert!(supersedes.iter().any(|edge| {
            edge.from_node_id == "capability:public:content.comments.create-comment-draft:1.1.0"
                && edge.to_node_id
                    == "capability:public:content.comments.create-comment-draft:1.0.0"
        }));
    }

    #[test]
    fn graph_lookup_supports_all_and_public_views_and_direct_edges() {
        let mut capabilities = CapabilityRegistry::new();
        let events = EventRegistry::new();
        let mut workflows = WorkflowRegistry::new();

        assert!(
            capabilities
                .register(capability_registration(
                    RegistryScope::Public,
                    capability_contract(
                        "content.comments.create-comment-draft",
                        "1.0.0",
                        vec![],
                        vec![]
                    ),
                    ImplementationKind::Executable,
                    None,
                ))
                .is_ok()
        );
        assert!(
            capabilities
                .register(capability_registration(
                    RegistryScope::Public,
                    capability_contract(
                        "content.comments.publish-comment",
                        "1.0.0",
                        vec![],
                        vec![]
                    ),
                    ImplementationKind::Executable,
                    None,
                ))
                .is_ok()
        );
        assert!(
            capabilities
                .register(capability_registration(
                    RegistryScope::Private,
                    capability_contract(
                        "content.comments.create-comment-draft",
                        "1.0.0",
                        vec![],
                        vec![]
                    ),
                    ImplementationKind::Executable,
                    None,
                ))
                .is_ok()
        );
        assert!(
            workflows
                .register(
                    &capabilities,
                    workflow_registration(
                        RegistryScope::Public,
                        direct_workflow_definition("content.comments.direct-flow", "1.0.0"),
                    ),
                )
                .is_ok()
        );

        let snapshot =
            project_metadata_graph(&capabilities, &events, &workflows, "2026-03-30T00:00:00Z");

        assert_eq!(
            snapshot
                .find_node(
                    MetadataGraphLookupScope::All,
                    MetadataGraphNodeKind::Capability,
                    "content.comments.create-comment-draft",
                    "1.0.0",
                )
                .map(|node| node.scope),
            Some(RegistryScope::Private)
        );
        assert_eq!(
            snapshot
                .find_node(
                    MetadataGraphLookupScope::PublicOnly,
                    MetadataGraphNodeKind::Capability,
                    "content.comments.create-comment-draft",
                    "1.0.0",
                )
                .map(|node| node.scope),
            Some(RegistryScope::Public)
        );

        let outgoing =
            snapshot.outgoing_edges("workflow:public:content.comments.direct-flow:1.0.0");
        assert!(
            outgoing
                .iter()
                .all(|edge| edge.kind == MetadataGraphEdgeKind::Composes)
        );
        assert!(outgoing.iter().any(|edge| {
            edge.to_node_id == "capability:public:content.comments.create-comment-draft:1.0.0"
        }));
    }

    fn capability_registration(
        scope: RegistryScope,
        contract: CapabilityContract,
        implementation_kind: ImplementationKind,
        workflow_ref: Option<(&str, &str)>,
    ) -> CapabilityRegistration {
        CapabilityRegistration {
            scope,
            contract_path: format!(
                "registry/{}/{}/{}",
                scope_name(scope),
                contract.id,
                contract.version
            ) + "/contract.json",
            artifact: CapabilityArtifactRecord {
                artifact_ref: format!("artifact:{}:{}", contract.id, contract.version),
                implementation_kind,
                source: SourceReference {
                    kind: SourceKind::Local,
                    location: "examples".to_string(),
                },
                binary: (implementation_kind == ImplementationKind::Executable).then(|| {
                    BinaryReference {
                        format: BinaryFormat::Wasm,
                        location: "artifacts/example.wasm".to_string(),
                    }
                }),
                workflow_ref: workflow_ref.map(|(workflow_id, workflow_version)| {
                    crate::WorkflowReference {
                        workflow_id: workflow_id.to_string(),
                        workflow_version: workflow_version.to_string(),
                    }
                }),
                digests: ArtifactDigests {
                    source_digest: "source-digest".to_string(),
                    binary_digest: Some("binary-digest".to_string()),
                },
                provenance: RegistryProvenance {
                    source: "test".to_string(),
                    author: "traverse".to_string(),
                    created_at: "2026-03-30T00:00:00Z".to_string(),
                },
            },
            registered_at: "2026-03-30T00:00:00Z".to_string(),
            tags: vec!["comments".to_string()],
            composability: ComposabilityMetadata {
                kind: if implementation_kind == ImplementationKind::Workflow {
                    CompositionKind::Composite
                } else {
                    CompositionKind::Atomic
                },
                patterns: vec![CompositionPattern::Sequential],
                provides: vec!["comment".to_string()],
                requires: vec!["draft".to_string()],
            },
            governing_spec: "005-capability-registry".to_string(),
            validator_version: "graph-test".to_string(),
            contract,
        }
    }

    fn capability_contract(
        id: &str,
        version: &str,
        consumes: Vec<EventReference>,
        emits: Vec<EventReference>,
    ) -> CapabilityContract {
        let (namespace, name) = split_id(id);
        CapabilityContract {
            kind: "capability_contract".to_string(),
            schema_version: "1.0.0".to_string(),
            id: id.to_string(),
            namespace,
            name: name.clone(),
            version: version.to_string(),
            lifecycle: Lifecycle::Active,
            owner: Owner {
                team: "graph".to_string(),
                contact: "graph@example.com".to_string(),
            },
            summary: format!("Summary for {id}"),
            description: format!("Description for {id}"),
            inputs: SchemaContainer {
                schema: json!({"type": "object"}),
            },
            outputs: SchemaContainer {
                schema: json!({"type": "object"}),
            },
            preconditions: vec![Condition {
                id: "pre".to_string(),
                description: "pre".to_string(),
            }],
            postconditions: vec![Condition {
                id: "post".to_string(),
                description: "post".to_string(),
            }],
            side_effects: vec![SideEffect {
                kind: SideEffectKind::MemoryOnly,
                description: "memory".to_string(),
            }],
            emits,
            consumes,
            permissions: vec![],
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
            policies: vec![],
            dependencies: vec![],
            service_type: ServiceType::Stateless,
            permitted_targets: vec![
                ExecutionTarget::Local,
                ExecutionTarget::Cloud,
                ExecutionTarget::Edge,
                ExecutionTarget::Device,
            ],
            event_trigger: None,
            connector_requirements: Vec::new(),
            provenance: Provenance {
                source: ProvenanceSource::Greenfield,
                author: "graph".to_string(),
                created_at: "2026-03-30T00:00:00Z".to_string(),
                spec_ref: Some("002-capability-contracts".to_string()),
                adr_refs: vec![],
                exception_refs: vec![],
            },
            evidence: vec![],
        }
    }

    fn event_registration(scope: RegistryScope, contract: EventContract) -> EventRegistration {
        EventRegistration {
            scope,
            contract,
            contract_path: format!(
                "registry/{}/{}/1.0.0/contract.json",
                scope_name(scope),
                "event"
            ),
            registered_at: "2026-03-30T00:00:00Z".to_string(),
            governing_spec: "011-event-registry".to_string(),
            validator_version: "graph-test".to_string(),
        }
    }

    fn event_contract(id: &str, version: &str) -> EventContract {
        let (namespace, name) = split_id(id);
        EventContract {
            kind: "event_contract".to_string(),
            schema_version: "1.0.0".to_string(),
            id: id.to_string(),
            namespace,
            name: name.clone(),
            version: version.to_string(),
            lifecycle: Lifecycle::Active,
            owner: Owner {
                team: "graph".to_string(),
                contact: "graph@example.com".to_string(),
            },
            summary: format!("Summary for {id}"),
            description: format!("Description for {id}"),
            payload: EventPayload {
                schema: json!({"type": "object"}),
                compatibility: PayloadCompatibility::BackwardCompatible,
            },
            classification: EventClassification {
                domain: "content".to_string(),
                bounded_context: "comments".to_string(),
                event_type: EventType::Domain,
                tags: vec!["comments".to_string()],
            },
            publishers: vec![CapabilityReference {
                capability_id: "content.comments.create-comment-draft".to_string(),
                version: "1.0.0".to_string(),
            }],
            subscribers: vec![CapabilityReference {
                capability_id: "content.comments.publish-comment".to_string(),
                version: "1.0.0".to_string(),
            }],
            policies: vec![],
            tags: vec!["comments".to_string()],
            provenance: EventProvenance {
                source: EventProvenanceSource::Greenfield,
                author: "graph".to_string(),
                created_at: "2026-03-30T00:00:00Z".to_string(),
            },
            evidence: vec![],
        }
    }

    fn workflow_registration(
        scope: RegistryScope,
        definition: WorkflowDefinition,
    ) -> WorkflowRegistration {
        WorkflowRegistration {
            scope,
            definition,
            workflow_path: "workflows/example.json".to_string(),
            registered_at: "2026-03-30T00:00:00Z".to_string(),
            validator_version: "graph-test".to_string(),
        }
    }

    fn workflow_definition(id: &str, version: &str) -> WorkflowDefinition {
        WorkflowDefinition {
            kind: "workflow_definition".to_string(),
            schema_version: "1.0.0".to_string(),
            id: id.to_string(),
            name: "publish-comment-flow".to_string(),
            version: version.to_string(),
            lifecycle: Lifecycle::Active,
            owner: Owner {
                team: "graph".to_string(),
                contact: "graph@example.com".to_string(),
            },
            summary: "Workflow summary".to_string(),
            inputs: SchemaContainer {
                schema: json!({"type": "object"}),
            },
            outputs: SchemaContainer {
                schema: json!({"type": "object"}),
            },
            nodes: vec![
                WorkflowNode {
                    node_id: "create".to_string(),
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
                    node_id: "publish".to_string(),
                    capability_id: "content.comments.publish-comment".to_string(),
                    capability_version: "1.0.0".to_string(),
                    input: WorkflowNodeInput {
                        from_workflow_input: vec!["draft_id".to_string()],
                    },
                    output: WorkflowNodeOutput {
                        to_workflow_state: vec!["comment_id".to_string()],
                    },
                },
            ],
            edges: vec![WorkflowEdge {
                edge_id: "draft-created".to_string(),
                from: "create".to_string(),
                to: "publish".to_string(),
                trigger: WorkflowEdgeTrigger::Event,
                event: Some(EventReference {
                    event_id: "content.comments.comment-draft-created".to_string(),
                    version: "1.0.0".to_string(),
                }),
                predicate: None,
            }],
            start_node: "create".to_string(),
            terminal_nodes: vec!["publish".to_string()],
            tags: vec!["comments".to_string()],
            governing_spec: "007-workflow-registry-traversal".to_string(),
        }
    }

    fn direct_workflow_definition(id: &str, version: &str) -> WorkflowDefinition {
        let mut definition = workflow_definition(id, version);
        definition.edges = vec![WorkflowEdge {
            edge_id: "create-to-publish".to_string(),
            from: "create".to_string(),
            to: "publish".to_string(),
            trigger: WorkflowEdgeTrigger::Direct,
            event: None,
            predicate: None,
        }];
        definition
    }

    fn split_id(id: &str) -> (String, String) {
        let mut parts = id.rsplitn(2, '.');
        let name = parts.next().unwrap_or(id).to_string();
        let namespace = parts.next().unwrap_or_default().to_string();
        (namespace, name)
    }

    fn scope_name(scope: RegistryScope) -> &'static str {
        match scope {
            RegistryScope::Public => "public",
            RegistryScope::Private => "private",
        }
    }
}
