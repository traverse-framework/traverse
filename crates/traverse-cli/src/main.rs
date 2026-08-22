#![allow(clippy::default_trait_access, clippy::doc_markdown)]

mod app_events_websocket;
mod app_runtime_events;
mod capability_packages;
mod federation_operator;
mod grpc_event_transport;
mod http_api;
mod supply_chain;
mod telemetry;

use capability_packages::load_capability_package;
use federation_operator::{
    render_federation_peers, render_federation_status, render_federation_sync,
};
use semver::Version;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::env;
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::Component;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use traverse_contracts::{
    CapabilityContract, EventContract, EventValidationContext, ValidationContext, parse_contract,
    parse_event_contract, validate_contract, validate_event_contract,
};
use traverse_contracts::{Lifecycle, ViolationRecord, reference_connector_contracts};
use traverse_registry::{
    ApplicationManifestError, ApplicationManifestErrorCode, ApplicationManifestFailure,
    ApplicationRegistrationRequest, ApplicationRegistry, ArtifactDigests,
    ArtifactResolutionRequest, BinaryFormat, BinaryReference, CapabilityArtifactRecord,
    CapabilityRegistration, CapabilityRegistry, ComposabilityMetadata, CompositionKind,
    CompositionPattern, ConnectorActivationRequest, ConnectorRegistration, DiscoveryQuery,
    EventRegistration, EventRegistry, ExecutableArtifactCandidate, ImplementationKind,
    InstalledConnector, LookupScope, PublicRegistryCapabilityRecord, PublicRegistryIndex,
    RegistryBundle, RegistryComponentResolver, RegistryProvenance, RegistryReference,
    RegistryScope, ResolvedRegistryComponent, SourceKind, SourceReference, WorkflowReference,
    WorkflowRegistration, WorkflowRegistry, cache_verified_public_registry_bytes,
    load_application_bundle_manifest, load_application_bundle_manifest_with_resolver,
    load_registry_bundle, public_registry_cache_path, resolve_executable_artifact,
    validate_connector_activation, write_synced_public_registry_state,
};
use traverse_runtime::executor::{SUPPORTED_HOST_ABI_VERSION, verify_wasm_host_abi_bytes};
use traverse_runtime::{
    ArtifactRouter, LocalExecutionFailure, LocalExecutionFailureCode, LocalExecutionOutput,
    LocalExecutor, Runtime, RuntimeExecutionOutcome, RuntimeRequest, RuntimeResultStatus,
    RuntimeTrace, parse_runtime_request,
};

#[derive(Debug)]
enum Command {
    BundleInspect {
        manifest_path: PathBuf,
        json_output: bool,
    },
    BundleRegister {
        manifest_path: PathBuf,
        json_output: bool,
    },
    AppNew {
        app_id: String,
        register: bool,
        workspace_id: Option<String>,
    },
    AppValidate {
        manifest_path: PathBuf,
        workspace_id: Option<String>,
        json_output: bool,
    },
    AppRegister {
        manifest_path: PathBuf,
        workspace_id: String,
        json_output: bool,
    },
    AppActivate {
        manifest_path: PathBuf,
        workspace_id: String,
        host_activation_path: PathBuf,
        json_output: bool,
    },
    RegistrySync {
        workspace_id: String,
        json_output: bool,
        source_repo: Option<String>,
    },
    RegistryList {
        workspace_id: String,
        namespace: Option<String>,
        id_prefix: Option<String>,
        json_output: bool,
    },
    RegistrySearch {
        query: String,
        workspace_id: String,
        namespace: Option<String>,
        json_output: bool,
    },
    CapabilityPublish {
        contract_path: PathBuf,
        artifact_path: PathBuf,
        registry_repo_path: PathBuf,
        registry_repo_remote: Option<String>,
        json_output: bool,
        dry_run: bool,
    },
    ComponentNew {
        component_id: String,
    },
    CapabilityNew {
        capability_id: String,
    },
    CapabilityPackageInspect {
        manifest_path: PathBuf,
    },
    CapabilityPackageExecute {
        manifest_path: PathBuf,
        request_path: PathBuf,
    },
    WasmAbiVerify {
        wasm_paths: Vec<PathBuf>,
    },
    ArtifactVerify {
        artifact_path: PathBuf,
    },
    ArtifactSign {
        artifact_path: PathBuf,
    },
    FederationPeers {
        manifest_path: PathBuf,
    },
    FederationSync {
        manifest_path: PathBuf,
    },
    FederationStatus {
        manifest_path: PathBuf,
    },
    ExpeditionExecute {
        request_path: PathBuf,
        trace_output_path: Option<PathBuf>,
        json_output: bool,
        validate_only: bool,
    },
    CapabilityDiscover {
        manifest_path: PathBuf,
        json_output: bool,
    },
    CapabilityInspect {
        contract_path: PathBuf,
    },
    Event {
        contract_path: PathBuf,
    },
    EventValidateProduct {
        descriptor_path: PathBuf,
    },
    TraceInspect {
        trace_path: PathBuf,
    },
    WorkflowRegister {
        workflow_path: PathBuf,
        workspace_id: String,
    },
    WorkflowList {
        workspace_id: String,
    },
    WorkflowInspect {
        workflow_id: String,
        version: Option<String>,
        workspace_id: String,
    },
    Serve {
        bind_address: String,
        auth_mode: Option<String>,
        allow_unauthenticated: bool,
        allowed_origins: Vec<String>,
        render_mobile_qr: bool,
        grpc_bind_address: Option<String>,
        grpc_tls_cert_path: Option<PathBuf>,
        grpc_tls_key_path: Option<PathBuf>,
    },
    TelemetryEnable,
    TelemetryDisable,
}

#[derive(Debug)]
enum CliError {
    ExecutionFailed(String),
    ValidationFailed(String),
    RegistrationConflict(String),
    IoError(String),
    UsageError(String),
}

impl CliError {
    fn message(&self) -> &str {
        match self {
            CliError::ExecutionFailed(m)
            | CliError::ValidationFailed(m)
            | CliError::RegistrationConflict(m)
            | CliError::IoError(m)
            | CliError::UsageError(m) => m,
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    match parse_command(&args) {
        Ok(Command::Serve {
            bind_address,
            auth_mode,
            allow_unauthenticated,
            allowed_origins,
            render_mobile_qr,
            grpc_bind_address,
            grpc_tls_cert_path,
            grpc_tls_key_path,
        }) => {
            if let Err(error) = run_serve(
                bind_address,
                auth_mode,
                allow_unauthenticated,
                allowed_origins,
                render_mobile_qr,
                grpc_bind_address,
                grpc_tls_cert_path,
                grpc_tls_key_path,
            ) {
                eprintln!("{error}");
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Ok(command) => match run_command(command) {
            Ok(output) => {
                println!("{output}");
                ExitCode::SUCCESS
            }
            Err(CliError::ExecutionFailed(msg)) => {
                eprintln!("{msg}");
                ExitCode::from(1)
            }
            Err(CliError::ValidationFailed(msg)) => {
                eprintln!("{msg}");
                ExitCode::from(2)
            }
            Err(CliError::RegistrationConflict(msg)) => {
                eprintln!("{msg}");
                ExitCode::from(3)
            }
            Err(CliError::IoError(msg)) => {
                eprintln!("{msg}");
                ExitCode::from(4)
            }
            Err(CliError::UsageError(msg)) => {
                eprintln!("{msg}");
                ExitCode::from(5)
            }
        },
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(5)
        }
    }
}

#[allow(clippy::too_many_lines)]
fn run_command(command: Command) -> Result<String, CliError> {
    match command {
        Command::BundleInspect {
            manifest_path,
            json_output,
        } => inspect_bundle(&manifest_path, json_output),
        Command::BundleRegister {
            manifest_path,
            json_output,
        } => register_bundle(&manifest_path, json_output),
        Command::AppNew {
            app_id,
            register,
            workspace_id,
        } => app_new(&app_id, register, workspace_id.as_deref()),
        Command::AppValidate {
            manifest_path,
            workspace_id,
            json_output,
        } => app_validate(&manifest_path, workspace_id.as_deref(), json_output),
        Command::AppRegister {
            manifest_path,
            workspace_id,
            json_output,
        } => app_register(&manifest_path, &workspace_id, json_output),
        Command::AppActivate {
            manifest_path,
            workspace_id,
            host_activation_path,
            json_output,
        } => app_activate(
            &manifest_path,
            &workspace_id,
            &host_activation_path,
            json_output,
        ),
        command @ (Command::RegistrySync { .. }
        | Command::RegistryList { .. }
        | Command::RegistrySearch { .. }) => run_registry_command(command),
        Command::CapabilityPublish {
            contract_path,
            artifact_path,
            registry_repo_path,
            registry_repo_remote,
            json_output,
            dry_run,
        } => capability_publish(
            &contract_path,
            &artifact_path,
            &registry_repo_path,
            registry_repo_remote,
            json_output,
            dry_run,
        ),
        Command::ComponentNew { component_id } => component_new(&component_id),
        Command::CapabilityNew { capability_id } => capability_new(&capability_id),
        Command::Serve { .. } => Err(CliError::UsageError(usage())),
        Command::CapabilityPackageInspect { manifest_path } => {
            inspect_capability_package(&manifest_path)
        }
        Command::CapabilityPackageExecute {
            manifest_path,
            request_path,
        } => execute_capability_package(&manifest_path, &request_path),
        Command::WasmAbiVerify { wasm_paths } => verify_wasm_abi_imports(&wasm_paths),
        Command::ArtifactVerify { artifact_path } => verify_supply_chain_artifact(&artifact_path),
        Command::ArtifactSign { artifact_path } => sign_supply_chain_artifact(&artifact_path),
        Command::FederationPeers { manifest_path } => {
            render_federation_peers(&manifest_path).map_err(CliError::IoError)
        }
        Command::FederationSync { manifest_path } => {
            render_federation_sync(&manifest_path).map_err(CliError::IoError)
        }
        Command::FederationStatus { manifest_path } => {
            render_federation_status(&manifest_path).map_err(CliError::IoError)
        }
        Command::ExpeditionExecute {
            request_path,
            trace_output_path,
            json_output,
            validate_only,
        } => execute_expedition(
            &request_path,
            trace_output_path.as_deref(),
            json_output,
            validate_only,
        ),
        Command::CapabilityDiscover {
            manifest_path,
            json_output,
        } => discover_capabilities(&manifest_path, json_output),
        Command::CapabilityInspect { contract_path } => inspect_capability(&contract_path),
        command @ (Command::Event { .. } | Command::EventValidateProduct { .. }) => {
            run_event_command(command)
        }
        Command::TraceInspect { trace_path } => inspect_trace(&trace_path),
        Command::WorkflowRegister {
            workflow_path,
            workspace_id,
        } => workflow_register(&workflow_path, &workspace_id),
        Command::WorkflowList { workspace_id } => workflow_list(&workspace_id),
        Command::WorkflowInspect {
            workflow_id,
            version,
            workspace_id,
        } => workflow_inspect(&workflow_id, version.as_deref(), &workspace_id),
        Command::TelemetryEnable => telemetry::enable_telemetry()
            .map(|config| render_telemetry_state("enabled", &config))
            .map_err(CliError::IoError),
        Command::TelemetryDisable => telemetry::disable_telemetry()
            .map(|config| render_telemetry_state("disabled", &config))
            .map_err(CliError::IoError),
    }
}

fn render_telemetry_state(action: &str, config: &telemetry::TelemetryConfig) -> String {
    match &config.install_id {
        Some(install_id) => format!("telemetry {action} (install_id: {install_id})"),
        None => format!("telemetry {action}"),
    }
}

fn run_event_command(command: Command) -> Result<String, CliError> {
    match command {
        Command::Event { contract_path } => inspect_event(&contract_path),
        Command::EventValidateProduct { descriptor_path } => {
            validate_event_product(&descriptor_path)
        }
        _ => Err(CliError::UsageError(usage())),
    }
}

fn run_registry_command(command: Command) -> Result<String, CliError> {
    match command {
        Command::RegistrySync {
            workspace_id,
            json_output,
            source_repo,
        } => registry_sync(&workspace_id, json_output, source_repo),
        Command::RegistryList {
            workspace_id,
            namespace,
            id_prefix,
            json_output,
        } => registry_list(
            &workspace_id,
            namespace.as_deref(),
            id_prefix.as_deref(),
            json_output,
        ),
        Command::RegistrySearch {
            query,
            workspace_id,
            namespace,
            json_output,
        } => registry_search(&query, &workspace_id, namespace.as_deref(), json_output),
        _ => Err(CliError::UsageError(
            "expected registry command".to_string(),
        )),
    }
}

fn parse_command(args: &[String]) -> Result<Command, String> {
    // Handle global --help / help
    if args.get(1).map(String::as_str) == Some("--help")
        || args.get(1).map(String::as_str) == Some("help")
    {
        return Err(usage());
    }

    // Handle per-subcommand --help
    let family = args.get(1).map(String::as_str);
    let subcommand = args.get(2).map(String::as_str);
    let has_help_flag = args.iter().any(|a| a == "--help");

    if has_help_flag {
        return Err(subcommand_help(family, subcommand));
    }

    match (family, subcommand) {
        (Some("serve"), _) => parse_serve_command(args),
        (Some("app"), Some("new")) => parse_app_new_command(args),
        (Some("app"), Some("validate")) => parse_app_validate_command(args),
        (Some("app"), Some("register")) => parse_app_register_command(args),
        (Some("app"), Some("activate")) => parse_app_activate_command(args),
        (Some("registry"), Some("sync")) => parse_registry_sync_command(args),
        (Some("registry"), Some("list")) => parse_registry_list_command(args),
        (Some("registry"), Some("search")) => parse_registry_search_command(args),
        (Some("component"), Some("new")) => parse_component_new_command(args),
        (Some("capability"), Some("new")) => parse_capability_new_command(args),
        (Some("federation"), Some(_)) => parse_federation_command(args),
        (Some("capability-package"), Some("execute")) => {
            parse_capability_package_execute_command(args)
        }
        (Some("artifact"), Some("verify")) => parse_artifact_verify_command(args),
        (Some("artifact"), Some("sign")) => parse_artifact_sign_command(args),
        (Some("wasm"), Some("abi")) => parse_wasm_abi_command(args),
        (Some("expedition"), Some("execute")) => parse_expedition_execute_command(args),
        (Some("capability"), Some("discover")) => parse_capability_discover_command(args),
        (Some("capability"), Some("publish")) => parse_capability_publish_command(args),
        (Some("workflow"), Some(_)) => parse_workflow_command(args),
        (Some("telemetry"), Some("enable")) => Ok(Command::TelemetryEnable),
        (Some("telemetry"), Some("disable")) => Ok(Command::TelemetryDisable),
        (Some("telemetry"), _) => Err(usage()),
        _ => parse_fixed_arity_command(args),
    }
}

fn subcommand_help(family: Option<&str>, subcommand: Option<&str>) -> String {
    match (family, subcommand) {
        (Some("bundle"), Some("inspect")) => help_bundle_inspect(),
        (Some("bundle"), Some("register")) => help_bundle_register(),
        (Some("bundle"), _) => help_bundle(),
        (Some("app"), Some("new")) => help_app_new(),
        (Some("app"), Some("validate")) => help_app_validate(),
        (Some("app"), Some("register")) => help_app_register(),
        (Some("app"), Some("activate")) => help_app_activate(),
        (Some("app"), _) => help_app(),
        (Some("registry"), Some("sync")) => help_registry_sync(),
        (Some("registry"), Some("list")) => help_registry_list(),
        (Some("registry"), Some("search")) => help_registry_search(),
        (Some("registry"), _) => help_registry(),
        (Some("component"), Some("new")) => help_component_new(),
        (Some("component"), _) => help_component(),
        (Some("capability-package"), Some("inspect")) => help_capability_package_inspect(),
        (Some("capability-package"), Some("execute")) => help_capability_package_execute(),
        (Some("capability-package"), _) => help_capability_package(),
        (Some("artifact"), Some("verify")) => help_artifact_verify(),
        (Some("artifact"), Some("sign")) => help_artifact_sign(),
        (Some("artifact"), _) => help_artifact(),
        (Some("wasm"), Some("abi")) => help_wasm_abi(),
        (Some("wasm"), _) => help_wasm(),
        (Some("workflow"), Some("register")) => help_workflow_register(),
        (Some("workflow"), Some("list")) => help_workflow_list(),
        (Some("workflow"), Some("inspect")) => help_workflow_inspect(),
        (Some("workflow"), _) => help_workflow(),
        (Some("expedition"), Some("execute")) => help_expedition_execute(),
        (Some("expedition"), _) => help_expedition(),
        (Some("capability"), Some("new")) => help_capability_new(),
        (Some("capability"), Some("inspect")) => help_capability_inspect(),
        (Some("capability"), Some("discover")) => help_capability_discover(),
        (Some("capability"), Some("publish")) => help_capability_publish(),
        (Some("capability"), _) => help_capability(),
        (Some("event"), Some("inspect")) => help_event_inspect(),
        (Some("event"), Some("validate-product")) => help_event_validate_product(),
        (Some("event"), _) => help_event(),
        (Some("trace"), Some("inspect")) => help_trace_inspect(),
        (Some("trace"), _) => help_trace(),
        (Some("serve"), _) => help_serve(),
        (Some("telemetry"), Some("enable")) => help_telemetry_enable(),
        (Some("telemetry"), Some("disable")) => help_telemetry_disable(),
        (Some("telemetry"), _) => help_telemetry(),
        _ => usage(),
    }
}

fn help_telemetry_enable() -> String {
    "traverse-cli telemetry enable

  Purpose:
    Opt in to anonymous usage telemetry: how often published capabilities are
    resolved and executed, reported to the Traverse maintainers. Off by
    default. Never shown as an interactive prompt anywhere else -- this
    command is the only way to turn it on.

    On first enable, generates and persists a random local install ID (a v4
    UUID, not derived from any machine-identifying value). Running enable
    again does not regenerate it.

    Each reported event contains exactly: event type (resolve/execute), the
    capability reference (namespace/id@version), a timestamp, and the
    install ID. Nothing else -- no CLI version, OS, hostname, or IP address.

  Optional flags:
    --help   Print this help text.

  Example:
    traverse-cli telemetry enable"
        .to_string()
}

fn help_telemetry_disable() -> String {
    "traverse-cli telemetry disable

  Purpose:
    Opt back out of anonymous usage telemetry. The no-op sink is wired
    immediately; no further usage events are ever sent. The install ID from
    a prior enable is retained, so a later enable does not mint a new one.

  Optional flags:
    --help   Print this help text.

  Example:
    traverse-cli telemetry disable"
        .to_string()
}

fn help_telemetry() -> String {
    "traverse-cli telemetry <subcommand>

  Subcommands:
    enable    Opt in to anonymous usage telemetry.
    disable   Opt out of anonymous usage telemetry (the default).

  Run `traverse-cli telemetry <subcommand> --help` for subcommand-specific help."
        .to_string()
}

fn help_app_new() -> String {
    "traverse-cli app new <app-id> [--register --workspace <workspace-id>]

  Purpose:
    Create a governed Traverse app bundle directory under apps/<app-id>.
    The scaffold contains a schema-valid application manifest, workspace-local
    config template, component reference directory, workflow directory, and
    bundle README. It contains no executable product behavior.

  Required arguments:
    <app-id>             Application id to scaffold.

  Optional flags:
    --register           Validate and attempt registration after generation.
    --workspace <id>     Workspace id for --register.
    --help               Print this help text.

  Example:
    traverse-cli app new youaskm3"
        .to_string()
}

fn help_app_validate() -> String {
    "traverse-cli app validate --manifest <path> [--workspace <workspace-id>] --json

  Purpose:
    Validate a downstream application manifest, component manifests,
    capability contracts, workflow references, WASM digests, workspace config,
    runtime constraints, public surfaces, and delegated model dependency
    declarations. Emits deterministic JSON setup evidence and does not
    register workspace state.

  Required flags:
    --manifest <path>   Path to the application manifest JSON file.
    --json              Emit machine-readable validation evidence.

  Optional flags:
    --workspace <id>    Resolve registry-ref components from this locally synced workspace.
    --help              Print this help text.

  Example:
    traverse-cli app validate \\
      --manifest examples/applications/expedition-readiness/app.manifest.json \\
      --json"
        .to_string()
}

fn help_app_register() -> String {
    "traverse-cli app register --manifest <path> --workspace <workspace-id> --json

  Purpose:
    Validate a downstream application manifest and atomically record durable
    local workspace registration state for later Traverse runtime loading.
    Emits deterministic JSON setup evidence and never exposes secret config
    values.

  Required flags:
    --manifest <path>   Path to the application manifest JSON file.
    --workspace <id>    Local workspace id to register into.
    --json              Emit machine-readable registration evidence.

  Optional flags:
    --help              Print this help text.

  Example:
    traverse-cli app register \\
      --manifest examples/applications/expedition-readiness/app.manifest.json \\
      --workspace local \\
      --json"
        .to_string()
}

fn help_app_activate() -> String {
    "traverse-cli app activate --manifest <path> --workspace <workspace-id> --host-activation <path> --json

  Purpose:
    Resolve and validate each declared application connector binding and
    required executable capability artifact against host-local metadata.
    Persists only immutable, non-secret activation evidence; configuration
    values are never emitted or written to workspace state.

  Required flags:
    --manifest <path>          Path to the application manifest JSON file.
    --workspace <id>           Local workspace id.
    --host-activation <path>   Host-private activation input JSON file.
    --json                     Emit machine-readable activation evidence.

  Example:
    traverse-cli app activate \\
      --manifest app.manifest.json \\
      --workspace local \\
      --host-activation host-activation.json \\
      --json"
        .to_string()
}

fn help_app() -> String {
    "traverse-cli app <subcommand> [options]

  Subcommands:
    new <app-id>                 Create a governed Traverse app bundle scaffold.
    validate --manifest <path>   Validate an app bundle and emit JSON evidence.
    register --manifest <path>   Validate and persist local app registration.
    activate --manifest <path>   Validate host connectors and artifacts, then persist evidence.

  Run `traverse-cli app <subcommand> --help` for subcommand-specific help."
        .to_string()
}

fn help_registry_sync() -> String {
    "traverse-cli registry sync --workspace <workspace-id> --json [--source-repo <owner/repo>]

  Purpose:
    Fetch the latest public registry index from traverse-framework/registry (or
    a configured alternate source) and atomically persist it as local workspace
    public-tier registry state. Runtime execution reads local state only and
    never live-fetches the registry.

  Required flags:
    --workspace <id>   Local workspace id to sync into.
    --json             Emit machine-readable sync evidence.

  Optional flags:
    --source-repo <owner/repo>  Sync from this GitHub repo instead of
                                traverse-framework/registry. The repo must adopt
                                the same capabilities/<namespace>/<id>/<version>/
                                contract.json layout and index-release CI -- this
                                is how a team stands up its own private registry
                                (own fork or clone), the same mechanism as the
                                public one, just a different source. If the repo
                                is private, set TRAVERSE_REGISTRY_TOKEN in the
                                environment to a token with read access to it.
    --help                      Print this help text.

  Example:
    traverse-cli registry sync --workspace local-default --json
    traverse-cli registry sync --workspace local-default --json \\
      --source-repo acme-corp/internal-registry"
        .to_string()
}

fn help_registry_list() -> String {
    "traverse-cli registry list --workspace <workspace-id> [--namespace <value>] [--id-prefix <value>] [--json]

  Purpose:
    List capability pointers from the locally synced public registry index.
    This command never contacts the network.

  Required flags:
    --workspace <id>   Local workspace containing synced registry state.

  Optional flags:
    --namespace <id>   Restrict results to one namespace.
    --id-prefix <id>   Restrict results to capability IDs with this prefix.
    --json             Emit machine-readable discovery evidence.

  Example:
    traverse-cli registry list --workspace local-default --json"
        .to_string()
}

fn help_registry_search() -> String {
    "traverse-cli registry search <query> --workspace <workspace-id> [--namespace <value>] [--json]

  Purpose:
    Search capability namespace and ID fields in the locally synced public
    registry index. This command never fetches contracts or contacts the network.

  Required arguments and flags:
    <query>            Case-insensitive substring to search.
    --workspace <id>   Local workspace containing synced registry state.

  Optional flags:
    --namespace <id>   Restrict results to one namespace.
    --json             Emit machine-readable discovery evidence.

  Example:
    traverse-cli registry search process --workspace local-default --json"
        .to_string()
}

fn help_registry() -> String {
    "traverse-cli registry <subcommand> [options]

  Subcommands:
    sync --workspace <id> --json   Sync the public registry index locally.
    list --workspace <id>           List locally synced capability pointers.
    search <query> --workspace <id> Search locally synced capability pointers.

  Run `traverse-cli registry <subcommand> --help` for subcommand-specific help."
        .to_string()
}

fn help_capability_publish() -> String {
    "traverse-cli capability publish --contract <path> --artifact <path> --registry-repo <path> --json [--dry-run] [--registry-repo-remote <owner/repo>]

  Purpose:
    Validate a capability contract and artifact, prepare the publication
    candidate under a local registry checkout, and open a human-reviewed
    registry PR. The command never publishes directly.

  Required flags:
    --contract <path>       Capability contract JSON to publish.
    --artifact <path>       Capability artifact used for digest verification.
    --registry-repo <path>  Local checkout of the target registry repo (a
                            clone of traverse-framework/registry, or of a
                            private fork/clone of it).
    --json                  Emit machine-readable publish evidence.

  Optional flags:
    --dry-run                       Validate (including persona_ref resolution against
                                    the registry personas tree) and report planned
                                    branch/path without writes.
    --registry-repo-remote <owner/repo>  Open the PR against this GitHub repo instead
                                    of traverse-framework/registry -- the counterpart
                                    to --registry-repo, since the local checkout path
                                    and the remote it should PR against are two
                                    separate things. Set this to the same repo your
                                    --registry-repo checkout's origin points at when
                                    publishing to your own private registry.
    --help                          Print this help text.

  Example:
    traverse-cli capability publish \\
      --contract contracts/examples/traverse-starter/capabilities/process/contract.json \\
      --artifact artifacts/process-agent.wasm \\
      --registry-repo ../registry \\
      --json

    traverse-cli capability publish \\
      --contract contracts/examples/traverse-starter/capabilities/process/contract.json \\
      --artifact artifacts/process-agent.wasm \\
      --registry-repo ../internal-registry \\
      --registry-repo-remote acme-corp/internal-registry \\
      --json"
        .to_string()
}

fn help_component_new() -> String {
    "traverse-cli component new <component-id>

  Retired (spec 100-capability-package-authoring FR-008):
    This command no longer scaffolds a package. It exits non-zero and
    directs you to the real create path.

  Use instead:
    traverse-cli capability new <capability-id>

  Run `traverse-cli capability new --help` for details."
        .to_string()
}

fn help_component() -> String {
    "traverse-cli component <subcommand> [options]

  Subcommands:
    new <component-id>   Retired — redirects to `capability new` (spec 100).

  Run `traverse-cli capability new --help` for the real create path."
        .to_string()
}

fn help_bundle_inspect() -> String {
    "traverse-cli bundle inspect <manifest-path>

  Purpose:
    Validate and summarize a registry bundle manifest. Reads the manifest JSON,
    resolves all declared capability/event/workflow artifact paths, and prints a
    structured summary of the bundle without registering anything.

  Required arguments:
    <manifest-path>   Path to the registry bundle manifest.json file.

  Optional flags:
    --help            Print this help text.

  Example:
    traverse-cli bundle inspect examples/expedition/registry-bundle/manifest.json"
        .to_string()
}

fn help_bundle_register() -> String {
    "traverse-cli bundle register <manifest-path>

  Purpose:
    Load a registry bundle and register its capabilities, events, and workflows
    into in-memory registries. Validates all artifact contracts and reports the
    set of records that would be committed.

  Required arguments:
    <manifest-path>   Path to the registry bundle manifest.json file.

  Optional flags:
    --help            Print this help text.

  Example:
    traverse-cli bundle register examples/expedition/registry-bundle/manifest.json"
        .to_string()
}

fn help_bundle() -> String {
    "traverse-cli bundle <subcommand> [options]

  Subcommands:
    inspect <manifest-path>    Validate and summarize a bundle manifest.
    register <manifest-path>   Register bundle artifacts into in-memory registries.

  Run `traverse-cli bundle <subcommand> --help` for subcommand-specific help."
        .to_string()
}

fn help_capability_package_inspect() -> String {
    "traverse-cli capability-package inspect <manifest-path>

  Purpose:
    Load and summarize a governed WASM capability package manifest. Verifies the
    binary digest, resolves the capability contract, and prints package metadata
    including model dependencies and workflow references.

  Required arguments:
    <manifest-path>   Path to the capability package manifest.json file.

  Optional flags:
    --help            Print this help text.

  Example:
    traverse-cli capability-package inspect examples/capabilities/expedition-intent-agent/manifest.json"
        .to_string()
}

fn help_capability_package_execute() -> String {
    "traverse-cli capability-package execute <manifest-path> <request-path>

  Purpose:
    Load a governed WASM capability package and execute it against a runtime request.
    Validates the package binary digest, registers the capability, and runs the
    request through the Traverse runtime.

  Required arguments:
    <manifest-path>   Path to the capability package manifest.json file.
    <request-path>    Path to the runtime request JSON file.

  Optional flags:
    --help            Print this help text.

  Example:
    traverse-cli capability-package execute \\
      examples/capabilities/expedition-intent-agent/manifest.json \\
      examples/capabilities/runtime-requests/interpret-expedition-intent.json"
        .to_string()
}

fn help_capability_package() -> String {
    "traverse-cli capability-package <subcommand> [options]

  Subcommands:
    inspect <manifest-path>                      Summarize a governed capability package.
    execute <manifest-path> <request-path>       Execute a capability package against a runtime request.

  Run `traverse-cli capability-package <subcommand> --help` for subcommand-specific help."
        .to_string()
}

fn help_artifact_verify() -> String {
    "traverse-cli artifact verify <artifact-or-manifest-path>

  Purpose:
    Verify one governed artifact's supply-chain evidence. The command reads
    either a manifest JSON path or an artifact path with sidecars named
    <artifact>.manifest.json and <artifact>.provenance.json, then emits a
    structured JSON report for checksum, signature, and provenance checks.

  Required arguments:
    <artifact-or-manifest-path>   Artifact file or artifact manifest JSON path.

  Optional flags:
    --help                       Print this help text.

  Example:
    traverse-cli artifact verify target/release/traverse-cli"
        .to_string()
}

fn help_artifact_sign() -> String {
    "traverse-cli artifact sign <artifact-path>

  Purpose:
    Sign one artifact with a freshly derived, single-use Ed25519 keypair and
    write a <artifact>.manifest.json sidecar that `artifact verify` can
    check. The signing key is derived from the artifact's own checksum and
    the current time, not a persistent release key — this proves the
    sign/verify round trip is internally consistent, it does not produce a
    publicly trusted release signature. Emits a structured JSON report to
    stdout describing what was signed and where the manifest sidecar landed.

  Required arguments:
    <artifact-path>   Artifact file to sign.

  Optional flags:
    --help            Print this help text.

  Example:
    traverse-cli artifact sign target/release/traverse-cli"
        .to_string()
}

fn help_artifact() -> String {
    "traverse-cli artifact <subcommand> [options]

  Subcommands:
    verify <artifact-or-manifest-path>   Verify checksum, signature, and provenance evidence.
    sign <artifact-path>                 Sign an artifact and write its manifest sidecar.

  Run `traverse-cli artifact verify --help` or `traverse-cli artifact sign --help` for subcommand-specific help."
        .to_string()
}

fn help_wasm_abi() -> String {
    "traverse-cli wasm abi verify <wasm-path>...

  Purpose:
    Validate one or more compiled WASM artifacts against the Traverse Host ABI
    v1 import whitelist before publication. Fails if any artifact imports a
    host function outside the governed ABI surface.

  Required arguments:
    <wasm-path>...   One or more .wasm files to validate.

  Optional flags:
    --help           Print this help text.

  Example:
    traverse-cli wasm abi verify examples/hello-world/say-hello-agent/artifacts/say-hello-agent.wasm"
        .to_string()
}

fn help_wasm() -> String {
    "traverse-cli wasm <subcommand> [options]

  Subcommands:
    abi verify <wasm-path>...   Validate WASM host imports against Traverse Host ABI v1.

  Run `traverse-cli wasm abi --help` for subcommand-specific help."
        .to_string()
}

fn help_workflow_register() -> String {
    "traverse-cli workflow register <workflow-path> [--workspace-id <id>]

  Purpose:
    Register a workflow definition via the HTTP/JSON API handler
    (POST /v1/workflows/register). This uses the same canonical workflow
    validation and immutability rules as the server.

  Required arguments:
    <workflow-path>       Path to the workflow definition JSON file.

  Optional flags:
    --workspace-id <id>   Workspace identifier (default: system).
    --help                Print this help text.

  Example:
    traverse-cli workflow register workflows/examples/hello-world/say-hello/workflow.json"
        .to_string()
}

fn help_workflow_list() -> String {
    "traverse-cli workflow list [--workspace-id <id>]

  Purpose:
    List registered workflows in a workspace via GET /v1/workflows.

  Optional flags:
    --workspace-id <id>   Workspace identifier (default: system).
    --help                Print this help text.

  Example:
    traverse-cli workflow list"
        .to_string()
}

fn help_workflow_inspect() -> String {
    "traverse-cli workflow inspect <workflow-id> [--version <v>] [--workspace-id <id>]

  Purpose:
    Inspect a registered workflow via GET /v1/workflows/{id}.

  Required arguments:
    <workflow-id>         Workflow identifier.

  Optional flags:
    --version <v>         Workflow version (default: latest in workspace).
    --workspace-id <id>   Workspace identifier (default: system).
    --help                Print this help text.

  Example:
    traverse-cli workflow inspect expedition.planning.plan-expedition"
        .to_string()
}

fn help_workflow() -> String {
    "traverse-cli workflow <subcommand> [options]

  Subcommands:
    register <workflow-path>   Register a workflow definition.
    list                       List registered workflows.
    inspect <workflow-id>      Inspect a registered workflow.

  Run `traverse-cli workflow inspect --help` for subcommand-specific help."
        .to_string()
}

fn help_expedition_execute() -> String {
    "traverse-cli expedition execute <request-path> [--trace-out <trace-path>]

  Purpose:
    Execute the canonical expedition workflow through the Traverse runtime.
    Loads the built-in expedition registry bundle, runs the request, and prints
    a structured execution summary. Optionally writes the full runtime trace to
    a JSON file for later inspection with `trace inspect`.

    Execution honesty: this command runs each of the five expedition-planning
    capabilities' real, checked-in WASM artifact through ArtifactRouter — the
    same execution path `traverse-cli capability-package execute` and `serve`
    use — chained end to end through the plan-expedition workflow. This
    command only recognizes the capabilities registered in the canonical
    expedition bundle; an unregistered capability ID fails closed.

  Required arguments:
    <request-path>          Path to the runtime request JSON file.

  Optional flags:
    --trace-out <path>      Write the runtime trace artifact to this path.
    --help                  Print this help text.

  Example:
    traverse-cli expedition execute \\
      examples/expedition/runtime-requests/plan-expedition.json \\
      --trace-out target/traces/plan-expedition.json"
        .to_string()
}

fn help_expedition() -> String {
    "traverse-cli expedition <subcommand> [options]

  Subcommands:
    execute <request-path> [--trace-out <path>]  Run the expedition workflow.

  Run `traverse-cli expedition execute --help` for subcommand-specific help."
        .to_string()
}

fn help_capability_inspect() -> String {
    "traverse-cli capability inspect <contract-path>

  Purpose:
    Parse and validate a capability contract file. Prints contract metadata
    including id, version, lifecycle, input/output schema references, and
    provenance information.

  Required arguments:
    <contract-path>   Path to the capability contract JSON file.

  Optional flags:
    --help            Print this help text.

  Example:
    traverse-cli capability inspect \\
      contracts/examples/expedition/capabilities/capture-expedition-objective/contract.json"
        .to_string()
}

fn help_capability_discover() -> String {
    "traverse-cli capability discover <manifest-path> [--json]

  Purpose:
    Load a registry bundle and list all discovered capabilities from the
    in-memory registry. Outputs capability IDs and versions in human-readable
    or JSON format.

  Required arguments:
    <manifest-path>   Path to the registry bundle manifest.json file.

  Optional flags:
    --json            Output structured JSON instead of human-readable text.
    --help            Print this help text.

  Example:
    traverse-cli capability discover examples/expedition/registry-bundle/manifest.json
    traverse-cli capability discover examples/expedition/registry-bundle/manifest.json --json"
        .to_string()
}

fn help_capability_new() -> String {
    "traverse-cli capability new <capability-id>

  Purpose:
    Create a governed WASM capability package directory under
    capabilities/<capability-id> (spec 100-capability-package-authoring).
    The scaffold is a real `kind: capability_package` — the same shape
    `capability-package inspect`/`execute` load in production — with a
    contract carrying authorable input/output fields (not empty
    placeholders) and a #![no_std] WASI guest stub. It does not claim to be
    executable yet: no wasm artifact exists until you build one.

  Required arguments:
    <capability-id>   Capability id to scaffold (dot-separated, e.g.
                       example.domain.my-cap).

  Optional flags:
    --help            Print this help text.

  Example:
    traverse-cli capability new example.domain.my-cap"
        .to_string()
}

fn help_capability() -> String {
    "traverse-cli capability <subcommand> [options]

  Subcommands:
    new <capability-id>             Scaffold a governed WASM capability package.
    inspect <contract-path>         Parse and validate a capability contract.
    discover <manifest-path>        List capabilities from a registry bundle.
    publish --contract <path>       Open a governed registry publication PR.

  Run `traverse-cli capability <subcommand> --help` for subcommand-specific help."
        .to_string()
}

fn help_event_inspect() -> String {
    "traverse-cli event inspect <contract-path>

  Purpose:
    Parse and validate an event contract file. Prints the event id, version,
    lifecycle, classification (domain/event-type), publisher and subscriber
    capability bindings, and tags.

  Required arguments:
    <contract-path>   Path to the event contract JSON file.

  Optional flags:
    --help            Print this help text.

  Example:
    traverse-cli event inspect \\
      contracts/examples/expedition/events/expedition-objective-captured/contract.json"
        .to_string()
}

fn help_event_validate_product() -> String {
    "traverse-cli event validate-product <descriptor-path>

  Purpose:
    Validate an ECCA event-product descriptor JSON document against the
    registry 0.11.0 descriptor rules (support route, field classifications,
    CloudEvents mapping, delivery semantics, retention).

  Required arguments:
    <descriptor-path>   Path to the EventProductDescriptor JSON file.

  Optional flags:
    --help              Print this help text.

  Example:
    traverse-cli event validate-product \\
      crates/traverse-runtime/tests/fixtures/ecca-event-products/valid.json"
        .to_string()
}

fn help_event() -> String {
    "traverse-cli event <subcommand> [options]

  Subcommands:
    inspect <contract-path>             Parse and validate an event contract.
    validate-product <descriptor-path>  Validate an ECCA event-product descriptor.

  Run `traverse-cli event <subcommand> --help` for subcommand-specific help."
        .to_string()
}

fn help_trace_inspect() -> String {
    "traverse-cli trace inspect <trace-path>

  Purpose:
    Parse and summarize a runtime trace artifact produced by `expedition execute
    --trace-out`. Prints trace metadata, state-machine validation results, the
    candidate collection summary, the selected capability, and the terminal state
    transition.

  Required arguments:
    <trace-path>   Path to the runtime trace JSON file.

  Optional flags:
    --help         Print this help text.

  Example:
    traverse-cli trace inspect target/traces/plan-expedition.json"
        .to_string()
}

fn help_trace() -> String {
    "traverse-cli trace <subcommand> [options]

  Subcommands:
    inspect <trace-path>   Parse and summarize a runtime trace artifact.

  Run `traverse-cli trace inspect --help` for subcommand-specific help."
        .to_string()
}

fn parse_app_new_command(args: &[String]) -> Result<Command, String> {
    let register = args.iter().any(|a| a == "--register");
    let workspace_id = parse_string_flag(args, "--workspace");
    let mut positional = Vec::new();
    let mut skip_next = false;

    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        match arg.as_str() {
            "--register" => {}
            "--workspace" => skip_next = true,
            _ => positional.push(arg),
        }
    }

    if args.iter().any(|a| a == "--workspace") && workspace_id.is_none() {
        return Err("--workspace requires a value".to_string());
    }

    match positional.as_slice() {
        [_, _, _, app_id] => Ok(Command::AppNew {
            app_id: (*app_id).clone(),
            register,
            workspace_id,
        }),
        _ => Err(usage()),
    }
}

fn parse_app_validate_command(args: &[String]) -> Result<Command, String> {
    let manifest_path = parse_string_flag(args, "--manifest")
        .ok_or_else(|| "app validate requires --manifest <path>".to_string())?;
    if !args.iter().any(|a| a == "--json") {
        return Err("app validate requires --json for stable setup evidence".to_string());
    }
    let workspace_id = parse_string_flag(args, "--workspace");
    if args.iter().any(|arg| arg == "--workspace") && workspace_id.is_none() {
        return Err("--workspace requires a value".to_string());
    }
    Ok(Command::AppValidate {
        manifest_path: PathBuf::from(manifest_path),
        workspace_id,
        json_output: true,
    })
}

fn parse_app_register_command(args: &[String]) -> Result<Command, String> {
    let manifest_path = parse_string_flag(args, "--manifest")
        .ok_or_else(|| "app register requires --manifest <path>".to_string())?;
    let workspace_id = parse_string_flag(args, "--workspace")
        .ok_or_else(|| "app register requires --workspace <workspace-id>".to_string())?;
    if !args.iter().any(|a| a == "--json") {
        return Err("app register requires --json for stable setup evidence".to_string());
    }
    Ok(Command::AppRegister {
        manifest_path: PathBuf::from(manifest_path),
        workspace_id,
        json_output: true,
    })
}

fn parse_app_activate_command(args: &[String]) -> Result<Command, String> {
    let manifest_path = parse_string_flag(args, "--manifest")
        .ok_or_else(|| "app activate requires --manifest <path>".to_string())?;
    let workspace_id = parse_string_flag(args, "--workspace")
        .ok_or_else(|| "app activate requires --workspace <workspace-id>".to_string())?;
    let host_activation_path = parse_string_flag(args, "--host-activation")
        .ok_or_else(|| "app activate requires --host-activation <path>".to_string())?;
    if !args.iter().any(|arg| arg == "--json") {
        return Err("app activate requires --json for stable activation evidence".to_string());
    }
    Ok(Command::AppActivate {
        manifest_path: PathBuf::from(manifest_path),
        workspace_id,
        host_activation_path: PathBuf::from(host_activation_path),
        json_output: true,
    })
}

fn parse_registry_sync_command(args: &[String]) -> Result<Command, String> {
    let workspace_id = parse_string_flag(args, "--workspace")
        .ok_or_else(|| "registry sync requires --workspace <workspace-id>".to_string())?;
    if !args.iter().any(|a| a == "--json") {
        return Err("registry sync requires --json for stable sync evidence".to_string());
    }
    Ok(Command::RegistrySync {
        workspace_id,
        json_output: true,
        source_repo: parse_string_flag(args, "--source-repo"),
    })
}

fn parse_registry_list_command(args: &[String]) -> Result<Command, String> {
    let workspace_id = parse_string_flag(args, "--workspace")
        .ok_or_else(|| "registry list requires --workspace <workspace-id>".to_string())?;
    Ok(Command::RegistryList {
        workspace_id,
        namespace: parse_string_flag(args, "--namespace"),
        id_prefix: parse_string_flag(args, "--id-prefix"),
        json_output: args.iter().any(|arg| arg == "--json"),
    })
}

fn parse_registry_search_command(args: &[String]) -> Result<Command, String> {
    let query = args
        .get(3)
        .filter(|value| !value.starts_with("--"))
        .cloned()
        .ok_or_else(|| "registry search requires <query>".to_string())?;
    let workspace_id = parse_string_flag(args, "--workspace")
        .ok_or_else(|| "registry search requires --workspace <workspace-id>".to_string())?;
    Ok(Command::RegistrySearch {
        query,
        workspace_id,
        namespace: parse_string_flag(args, "--namespace"),
        json_output: args.iter().any(|arg| arg == "--json"),
    })
}

fn parse_capability_publish_command(args: &[String]) -> Result<Command, String> {
    let contract_path = parse_string_flag(args, "--contract")
        .ok_or_else(|| "capability publish requires --contract <path>".to_string())?;
    let artifact_path = parse_string_flag(args, "--artifact")
        .ok_or_else(|| "capability publish requires --artifact <path>".to_string())?;
    let registry_repo_path = parse_string_flag(args, "--registry-repo")
        .ok_or_else(|| "capability publish requires --registry-repo <path>".to_string())?;
    if !args.iter().any(|a| a == "--json") {
        return Err("capability publish requires --json for stable publish evidence".to_string());
    }

    Ok(Command::CapabilityPublish {
        contract_path: PathBuf::from(contract_path),
        artifact_path: PathBuf::from(artifact_path),
        registry_repo_path: PathBuf::from(registry_repo_path),
        registry_repo_remote: parse_string_flag(args, "--registry-repo-remote"),
        json_output: true,
        dry_run: args.iter().any(|a| a == "--dry-run"),
    })
}

fn parse_component_new_command(args: &[String]) -> Result<Command, String> {
    match args {
        [_, _, _, component_id] => Ok(Command::ComponentNew {
            component_id: component_id.clone(),
        }),
        _ => Err(usage()),
    }
}

fn parse_capability_new_command(args: &[String]) -> Result<Command, String> {
    match args {
        [_, _, _, capability_id] => Ok(Command::CapabilityNew {
            capability_id: capability_id.clone(),
        }),
        _ => Err(usage()),
    }
}

fn help_serve() -> String {
    "traverse-cli serve [--bind <address>] [--port <port>] [--auth <mode>] [--allow-unauthenticated] [--qr] [--grpc-bind <address> --grpc-tls-cert <path> --grpc-tls-key <path>]

  Purpose:
    Start a development and CI HTTP/JSON API on 127.0.0.1:8787 by default.
    This is not the production app topology: production apps embed Traverse
    through the public embedder packages and require neither a loopback sidecar
    nor .traverse/server.json discovery. This command writes that file only for
    local development/CI discovery and exposes:
      GET  /healthz                    Returns the spec 033 health envelope.
      GET  /v1/capabilities            Returns JSON array of registered capabilities.
      POST /v1/capabilities/execute    Accepts RuntimeRequest JSON, returns trace + result.

    Loopback callers (127.0.0.1 / ::1) are allowed without authentication. All
    other callers must supply an Authorization: Bearer <token> header unless
    --allow-unauthenticated is set.

  Optional flags:
    --bind <address>           Address and port to listen on (default: 127.0.0.1:8787,
                               or 0.0.0.0:8787 with --auth dev-any).
    --port <N>                 Compatibility shortcut for --bind 127.0.0.1:<N>.
    --auth <mode>              Authentication mode: dev-loopback (default) or dev-any.
    --allow-origin <origin>    Allow an exact browser Origin, repeatable for
                               production web apps. Wildcard '*' is rejected.
    --allow-unauthenticated    Accept unauthenticated requests from non-loopback
                               addresses. Prints a warning to stderr. Unsafe in
                               production.
    --qr                       Print an ASCII QR code for the traverse://connect
                               mobile provisioning URL.
    --grpc-bind <address>      Start the TLS gRPC EventService on this address.
                               Requires --grpc-tls-cert and --grpc-tls-key.
    --grpc-tls-cert <path>     PEM certificate chain for the gRPC listener.
    --grpc-tls-key <path>      PEM private key for the gRPC listener.
    --help                     Print this help text.

  Example:
    traverse-cli serve
    traverse-cli serve --bind 127.0.0.1:9090
    traverse-cli serve --port 9090 --allow-unauthenticated"
        .to_string()
}

fn parse_serve_command(args: &[String]) -> Result<Command, String> {
    let allow_unauthenticated = args.iter().any(|a| a == "--allow-unauthenticated");
    let render_mobile_qr = args.iter().any(|a| a == "--qr");
    let bind_flag_pos = args.iter().position(|a| a == "--bind");
    let port_flag_pos = args.iter().position(|a| a == "--port");
    let auth_flag_pos = args.iter().position(|a| a == "--auth");
    let grpc_bind_address = parse_string_flag(args, "--grpc-bind");
    let grpc_tls_cert_path = parse_string_flag(args, "--grpc-tls-cert").map(PathBuf::from);
    let grpc_tls_key_path = parse_string_flag(args, "--grpc-tls-key").map(PathBuf::from);
    let mut allowed_origins = Vec::new();

    if bind_flag_pos.is_some() && port_flag_pos.is_some() {
        return Err("--bind and --port cannot be used together".to_string());
    }

    let auth_mode = if let Some(pos) = auth_flag_pos {
        let mode = args
            .get(pos + 1)
            .ok_or_else(|| "--auth requires a value".to_string())?;
        if !matches!(mode.as_str(), "dev-loopback" | "dev-any") {
            return Err("--auth value must be dev-loopback or dev-any".to_string());
        }
        Some(mode.clone())
    } else {
        None
    };

    for (idx, arg) in args.iter().enumerate() {
        if arg != "--allow-origin" {
            continue;
        }
        let origin = args
            .get(idx + 1)
            .ok_or_else(|| "--allow-origin requires a value".to_string())?;
        if origin == "*" {
            return Err("--allow-origin '*' is not allowed".to_string());
        }
        allowed_origins.push(origin.clone());
    }

    let bind_address = if let Some(pos) = bind_flag_pos {
        args.get(pos + 1)
            .ok_or_else(|| "--bind requires a value".to_string())?
            .clone()
    } else if let Some(pos) = port_flag_pos {
        let port = args
            .get(pos + 1)
            .ok_or_else(|| "--port requires a value".to_string())?
            .parse::<u16>()
            .map_err(|_| "--port value must be a valid port number (0-65535)".to_string())?;
        if auth_mode.as_deref() == Some("dev-any") {
            format!("0.0.0.0:{port}")
        } else {
            format!("127.0.0.1:{port}")
        }
    } else if auth_mode.as_deref() == Some("dev-any") {
        "0.0.0.0:8787".to_string()
    } else {
        "127.0.0.1:8787".to_string()
    };

    Ok(Command::Serve {
        bind_address,
        auth_mode,
        allow_unauthenticated,
        allowed_origins,
        render_mobile_qr,
        grpc_bind_address,
        grpc_tls_cert_path,
        grpc_tls_key_path,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_serve(
    bind_address: String,
    auth_mode: Option<String>,
    allow_unauthenticated: bool,
    allowed_origins: Vec<String>,
    render_mobile_qr: bool,
    grpc_bind_address: Option<String>,
    grpc_tls_cert_path: Option<PathBuf>,
    grpc_tls_key_path: Option<PathBuf>,
) -> Result<(), String> {
    let registered =
        load_registered_bundle(&canonical_expedition_bundle_path()).map_err(|e| e.to_string())?;

    let config = http_api::ApiServerConfig {
        bind_address,
        requested_auth_mode: auth_mode,
        allow_unauthenticated,
        allowed_origins,
        render_mobile_qr,
        capability_registry: registered.capability_registry,
        workflow_registry: registered.workflow_registry,
        registry_root: std::env::current_dir()
            .map_err(|e| format!("failed to resolve current directory: {e}"))?
            .join(".traverse/registry"),
        executor: ArtifactRouter::new().map_err(|error| error.message)?,
        idempotency_retention_seconds: None,
        jwt_verification_key_hex: std::env::var("TRAVERSE_JWT_VERIFICATION_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty()),
        read_timeout: None,
        write_timeout: None,
        request_deadline: None,
        max_concurrent_connections: None,
        grpc_bind_address,
        grpc_tls_cert_path,
        grpc_tls_key_path,
    };

    http_api::serve_http_api(config).map_err(|e| e.to_string())
}

fn parse_fixed_arity_command(args: &[String]) -> Result<Command, String> {
    let json_output = args.iter().any(|a| a == "--json");

    // Allow optional --json flag: 4 args (no flag) or 5 args (with --json)
    let positional_count = args.len() - usize::from(json_output);
    if positional_count != 4 {
        return Err(usage());
    }

    // Collect positional args (skip the --json flag)
    let positional: Vec<&String> = args.iter().filter(|a| a.as_str() != "--json").collect();

    match (positional[1].as_str(), positional[2].as_str()) {
        ("bundle", "inspect") => Ok(Command::BundleInspect {
            manifest_path: PathBuf::from(positional[3]),
            json_output,
        }),
        ("bundle", "register") => Ok(Command::BundleRegister {
            manifest_path: PathBuf::from(positional[3]),
            json_output,
        }),
        ("capability-package", "inspect") => Ok(Command::CapabilityPackageInspect {
            manifest_path: PathBuf::from(positional[3]),
        }),
        ("capability", "inspect") => Ok(Command::CapabilityInspect {
            contract_path: PathBuf::from(positional[3]),
        }),
        ("federation", "peers") => Ok(Command::FederationPeers {
            manifest_path: PathBuf::from(positional[3]),
        }),
        ("federation", "sync") => Ok(Command::FederationSync {
            manifest_path: PathBuf::from(positional[3]),
        }),
        ("federation", "status") => Ok(Command::FederationStatus {
            manifest_path: PathBuf::from(positional[3]),
        }),
        ("event", "inspect") => Ok(Command::Event {
            contract_path: PathBuf::from(positional[3]),
        }),
        ("event", "validate-product") => Ok(Command::EventValidateProduct {
            descriptor_path: PathBuf::from(positional[3]),
        }),
        ("trace", "inspect") => Ok(Command::TraceInspect {
            trace_path: PathBuf::from(positional[3]),
        }),
        _ => Err(usage()),
    }
}

fn parse_artifact_verify_command(args: &[String]) -> Result<Command, String> {
    match args {
        [_, _, _, artifact_path] => Ok(Command::ArtifactVerify {
            artifact_path: PathBuf::from(artifact_path),
        }),
        _ => Err(usage()),
    }
}

fn parse_artifact_sign_command(args: &[String]) -> Result<Command, String> {
    match args {
        [_, _, _, artifact_path] => Ok(Command::ArtifactSign {
            artifact_path: PathBuf::from(artifact_path),
        }),
        _ => Err(usage()),
    }
}

fn parse_capability_package_execute_command(args: &[String]) -> Result<Command, String> {
    match args {
        [_, _, _, manifest_path, request_path] => Ok(Command::CapabilityPackageExecute {
            manifest_path: PathBuf::from(manifest_path),
            request_path: PathBuf::from(request_path),
        }),
        _ => Err(usage()),
    }
}

fn parse_wasm_abi_command(args: &[String]) -> Result<Command, String> {
    match args {
        [_, _, abi, verify, wasm_paths @ ..] if abi == "abi" && verify == "verify" => {
            if wasm_paths.is_empty() {
                return Err(usage());
            }
            Ok(Command::WasmAbiVerify {
                wasm_paths: wasm_paths.iter().map(PathBuf::from).collect(),
            })
        }
        _ => Err(usage()),
    }
}

fn parse_federation_command(args: &[String]) -> Result<Command, String> {
    match args {
        [_, _, _, manifest_path] if args[2] == "peers" => Ok(Command::FederationPeers {
            manifest_path: PathBuf::from(manifest_path),
        }),
        [_, _, _, manifest_path] if args[2] == "sync" => Ok(Command::FederationSync {
            manifest_path: PathBuf::from(manifest_path),
        }),
        [_, _, _, manifest_path] if args[2] == "status" => Ok(Command::FederationStatus {
            manifest_path: PathBuf::from(manifest_path),
        }),
        _ => Err(usage()),
    }
}

fn parse_expedition_execute_command(args: &[String]) -> Result<Command, String> {
    let json_output = args.iter().any(|a| a == "--json");
    let validate_only = args.iter().any(|a| a == "--validate-only");

    // Collect positional args (skip --json and --validate-only flags)
    let positional: Vec<&String> = args
        .iter()
        .filter(|a| a.as_str() != "--json" && a.as_str() != "--validate-only")
        .collect();

    match positional.as_slice() {
        [_, _, _, request_path] => Ok(Command::ExpeditionExecute {
            request_path: PathBuf::from(*request_path),
            trace_output_path: None,
            json_output,
            validate_only,
        }),
        [_, _, _, request_path, flag, trace_output_path] if flag.as_str() == "--trace-out" => {
            Ok(Command::ExpeditionExecute {
                request_path: PathBuf::from(*request_path),
                trace_output_path: Some(PathBuf::from(*trace_output_path)),
                json_output,
                validate_only,
            })
        }
        _ => Err(usage()),
    }
}

fn parse_capability_discover_command(args: &[String]) -> Result<Command, String> {
    let json_output = args.iter().any(|a| a == "--json");
    let positional: Vec<&String> = args.iter().filter(|a| a.as_str() != "--json").collect();

    match positional.as_slice() {
        [_, _, _, manifest_path] => Ok(Command::CapabilityDiscover {
            manifest_path: PathBuf::from(*manifest_path),
            json_output,
        }),
        _ => Err(usage()),
    }
}

fn parse_workflow_command(args: &[String]) -> Result<Command, String> {
    let workspace_id = parse_string_flag(args, "--workspace-id")
        .or_else(|| std::env::var("TRAVERSE_WORKSPACE_ID").ok())
        .unwrap_or_else(|| "system".to_string());

    match args {
        [_, _, _, workflow_path, rest @ ..] if args[2] == "register" => {
            let override_workspace = parse_string_flag(rest, "--workspace-id");
            Ok(Command::WorkflowRegister {
                workflow_path: PathBuf::from(workflow_path),
                workspace_id: override_workspace.unwrap_or(workspace_id),
            })
        }
        [_, _, ..] if args[2] == "list" => Ok(Command::WorkflowList { workspace_id }),
        [_, _, _, workflow_id, rest @ ..] if args[2] == "inspect" => {
            let version = parse_string_flag(rest, "--version");
            let override_workspace = parse_string_flag(rest, "--workspace-id");
            Ok(Command::WorkflowInspect {
                workflow_id: workflow_id.clone(),
                version,
                workspace_id: override_workspace.unwrap_or(workspace_id),
            })
        }
        _ => Err(usage()),
    }
}

fn parse_string_flag(args: &[String], flag: &str) -> Option<String> {
    let pos = args.iter().position(|a| a == flag)?;
    args.get(pos + 1).cloned()
}

fn inspect_bundle(manifest_path: &Path, json_output: bool) -> Result<String, CliError> {
    let bundle = load_registry_bundle(manifest_path)
        .map_err(|failure| CliError::IoError(failure.errors[0].message.clone()))?;
    if json_output {
        let json = serde_json::json!({
            "bundle_id": bundle.bundle_id,
            "version": bundle.version,
            "scope": format!("{:?}", bundle.scope).to_lowercase(),
            "capabilities": bundle.capabilities.len(),
            "events": bundle.events.len(),
            "workflows": bundle.workflows.len(),
            "capability_ids": bundle.capabilities.iter().map(|c| format!("{}@{}", c.manifest.id, c.manifest.version)).collect::<Vec<_>>(),
            "event_ids": bundle.events.iter().map(|e| format!("{}@{}", e.manifest.id, e.manifest.version)).collect::<Vec<_>>(),
            "workflow_ids": bundle.workflows.iter().map(|w| format!("{}@{}", w.manifest.id, w.manifest.version)).collect::<Vec<_>>(),
        });
        serde_json::to_string_pretty(&json)
            .map_err(|e| CliError::IoError(format!("failed to serialize bundle summary: {e}")))
    } else {
        Ok(render_bundle_summary(&bundle))
    }
}

fn register_bundle(manifest_path: &Path, json_output: bool) -> Result<String, CliError> {
    let base_dir = env::current_dir().map_err(|error| {
        CliError::IoError(format!("failed to resolve current directory: {error}"))
    })?;
    let workspace_id = env::var("TRAVERSE_WORKSPACE_ID").unwrap_or_else(|_| "system".to_string());
    let public_state_path =
        traverse_registry::synced_public_registry_state_path(&base_dir, &workspace_id);
    let public_records = if public_state_path.exists() {
        traverse_registry::load_synced_public_registry_state(&base_dir, &workspace_id)
            .map_err(|failure| {
                CliError::ValidationFailed(render_public_registry_state_failure(failure))
            })?
            .capabilities
    } else {
        Vec::new()
    };
    let registered = load_registered_bundle_with_public_records(manifest_path, &public_records)?;
    if json_output {
        let json = serde_json::json!({
            "registered_capabilities": registered.capability_records.len(),
            "registered_events": registered.event_records.len(),
            "registered_workflows": registered.workflow_records.len(),
            "evidence": registered.evidence,
        });
        serde_json::to_string_pretty(&json).map_err(|e| {
            CliError::IoError(format!("failed to serialize registration summary: {e}"))
        })
    } else {
        Ok(render_bundle_registration_summary(
            &registered.bundle,
            &registered.capability_records,
            &registered.event_records,
            &registered.workflow_records,
            &registered.evidence,
        ))
    }
}

fn render_public_registry_state_failure(
    failure: traverse_registry::PublicRegistryStateFailure,
) -> String {
    failure
        .errors
        .into_iter()
        .map(|error| format!("{:?}: {} ({})", error.code, error.message, error.path))
        .collect::<Vec<_>>()
        .join("; ")
}

fn app_new(app_id: &str, register: bool, workspace_id: Option<&str>) -> Result<String, CliError> {
    let base_dir = env::current_dir()
        .map_err(|e| CliError::IoError(format!("failed to resolve current directory: {e}")))?;
    app_new_at(&base_dir, app_id, register, workspace_id)
}

fn app_new_at(
    base_dir: &Path,
    app_id: &str,
    register: bool,
    workspace_id: Option<&str>,
) -> Result<String, CliError> {
    validate_scaffold_id(app_id, "app id")?;
    let app_dir = base_dir.join("apps").join(app_id);
    if app_dir.exists() {
        return Err(CliError::IoError(format!(
            "app scaffold target already exists: {}",
            app_dir.display()
        )));
    }

    let components_dir = app_dir.join("components");
    let workflows_dir = app_dir.join("workflows");
    fs::create_dir_all(&components_dir).map_err(|e| {
        CliError::IoError(format!(
            "failed to create component reference directory {}: {e}",
            components_dir.display()
        ))
    })?;
    fs::create_dir_all(&workflows_dir).map_err(|e| {
        CliError::IoError(format!(
            "failed to create workflow directory {}: {e}",
            workflows_dir.display()
        ))
    })?;

    let manifest_path = app_dir.join("manifest.json");
    write_pretty_json(
        &manifest_path,
        &serde_json::json!({
            "app_id": app_id,
            "version": "1.0.0",
            "schema_version": "1.0.0",
            "workspace_defaults": {
                "workspace_id": format!("{app_id}-local"),
                "config_path": "workspace.config.json"
            },
            "components": [],
            "workflows": [],
            "model_dependencies": [],
            "config_schema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {}
            },
            "default_config": {},
            "placement_policy": {
                "preferred_targets": ["local"]
            },
            "public_surfaces": ["cli"]
        }),
    )?;
    write_pretty_json(
        &app_dir.join("workspace.config.json"),
        &serde_json::json!({
            "workspace_id": format!("{app_id}-local"),
            "overrides": {},
            "secrets": {}
        }),
    )?;
    write_new_file(
        &components_dir.join("README.md"),
        "Add component manifest references here after real component packages are authored.\n",
    )?;
    write_new_file(
        &workflows_dir.join("README.md"),
        "Add workflow definitions here after real component-backed workflows are authored.\n",
    )?;
    write_new_file(
        &app_dir.join("README.md"),
        &format!(
            "# {app_id}\n\nGoverned Traverse app bundle scaffold for `{app_id}`.\n\nThe initial bundle contains no executable components or workflows. Add real WASM component manifests, real capability contracts, workflow definitions, and verified WASM digests before registration.\n"
        ),
    )?;

    let mut lines = vec![
        format!("created_app: {app_id}"),
        format!("app_dir: {}", app_dir.display()),
        format!("manifest: {}", manifest_path.display()),
        format!(
            "workspace_config: {}",
            app_dir.join("workspace.config.json").display()
        ),
        format!("components_dir: {}", components_dir.display()),
        format!("workflows_dir: {}", workflows_dir.display()),
    ];

    if register {
        let workspace = workspace_id.unwrap_or(app_id);
        let registration = register_generated_app_bundle(app_id, workspace, &manifest_path)?;
        lines.push(registration);
    }

    Ok(lines.join("\n"))
}

/// Spec `100-capability-package-authoring` FR-008: `component new` no longer
/// emits the pre-Spec-100 empty, non-loadable scaffold. It redirects authors
/// to the real `capability new` create path instead.
fn component_new(_component_id: &str) -> Result<String, CliError> {
    Err(CliError::UsageError(
        "`traverse-cli component new` has been retired in favor of `traverse-cli capability new \
         <capability-id>`, which scaffolds a real, inspectable `kind: capability_package` \
         (spec 100-capability-package-authoring). Run `traverse-cli capability new --help` for \
         details."
            .to_string(),
    ))
}

fn capability_new(capability_id: &str) -> Result<String, CliError> {
    let base_dir = env::current_dir()
        .map_err(|e| CliError::IoError(format!("failed to resolve current directory: {e}")))?;
    capability_new_at(&base_dir, capability_id)
}

fn capability_new_at(base_dir: &Path, capability_id: &str) -> Result<String, CliError> {
    validate_scaffold_id(capability_id, "capability id")?;
    let capability_dir = base_dir.join("capabilities").join(capability_id);
    if capability_dir.exists() {
        return Err(CliError::IoError(format!(
            "capability scaffold target already exists: {}",
            capability_dir.display()
        )));
    }

    let src_dir = capability_dir.join("src");
    let artifacts_dir = capability_dir.join("artifacts");
    let runtime_requests_dir = capability_dir.join("runtime-requests");
    fs::create_dir_all(&src_dir).map_err(|e| {
        CliError::IoError(format!(
            "failed to create source directory {}: {e}",
            src_dir.display()
        ))
    })?;
    fs::create_dir_all(&artifacts_dir).map_err(|e| {
        CliError::IoError(format!(
            "failed to create artifact directory {}: {e}",
            artifacts_dir.display()
        ))
    })?;
    fs::create_dir_all(&runtime_requests_dir).map_err(|e| {
        CliError::IoError(format!(
            "failed to create runtime-requests directory {}: {e}",
            runtime_requests_dir.display()
        ))
    })?;

    let leaf_name = scaffold_leaf_name(capability_id);
    let wasm_name = format!("{leaf_name}.wasm");

    write_new_file(&src_dir.join("agent.rs"), &capability_guest_stub_source())?;
    write_new_file(
        &artifacts_dir.join("README.md"),
        "Place the compiled WASM artifact here after running build-fixture.sh. \
         That script writes the matching `binary.expected_digest` into manifest.json. \
         `capability-package inspect`/`execute` refuse to treat this package as executable \
         until a real artifact exists here with a matching digest.\n",
    )?;
    write_new_file(
        &capability_dir.join("build-fixture.sh"),
        &capability_build_fixture_script(&wasm_name),
    )?;
    write_pretty_json(
        &capability_dir.join("contract.json"),
        &capability_contract_json(capability_id, &leaf_name),
    )?;
    write_pretty_json(
        &capability_dir.join("manifest.json"),
        &capability_package_manifest_json(capability_id, &wasm_name),
    )?;
    write_pretty_json(
        &runtime_requests_dir.join(format!("{leaf_name}.json")),
        &capability_sample_runtime_request_json(capability_id, &leaf_name),
    )?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let build_script = capability_dir.join("build-fixture.sh");
        if let Ok(metadata) = fs::metadata(&build_script) {
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o755);
            let _ = fs::set_permissions(&build_script, permissions);
        }
    }

    let manifest_path = capability_dir.join("manifest.json");
    let request_path = runtime_requests_dir.join(format!("{leaf_name}.json"));
    Ok([
        format!("created_capability: {capability_id}"),
        format!("capability_dir: {}", capability_dir.display()),
        format!("manifest: {}", manifest_path.display()),
        format!(
            "contract: {}",
            capability_dir.join("contract.json").display()
        ),
        format!("source: {}", src_dir.join("agent.rs").display()),
        format!("sample_request: {}", request_path.display()),
        "next_steps:".to_string(),
        "  1. Implement real input/output fields and business logic in src/agent.rs \
             and contract.json (this scaffold is a placeholder, not executable yet)."
            .to_string(),
        format!(
            "  2. Build the WASM artifact (also writes binary.expected_digest): bash {}",
            capability_dir.join("build-fixture.sh").display()
        ),
        format!(
            "  3. Run `traverse-cli capability-package inspect {}`.",
            manifest_path.display()
        ),
        format!(
            "  4. Run `traverse-cli capability-package execute {} {}`.",
            manifest_path.display(),
            request_path.display()
        ),
    ]
    .join("\n"))
}

fn capability_package_manifest_json(capability_id: &str, wasm_name: &str) -> Value {
    serde_json::json!({
        "kind": "capability_package",
        "schema_version": "1.0.0",
        "package_id": capability_id,
        "version": "1.0.0",
        "summary": format!("Governed capability package for {capability_id}."),
        "capability_ref": {
            "id": capability_id,
            "version": "1.0.0",
            "contract_path": "contract.json"
        },
        "known_compositions": [
            {
                "workflow_id": capability_id,
                "workflow_version": "1.0.0"
            }
        ],
        "source": {
            "path": "src/agent.rs",
            "language": "rust",
            "entry": "run"
        },
        "binary": {
            "path": format!("artifacts/{wasm_name}"),
            "format": "wasm",
            "expected_digest": "fnv1a64:0000000000000000",
            "abi_version": SUPPORTED_HOST_ABI_VERSION
        },
        "constraints": {
            "host_api_access": "none",
            "network_access": "forbidden",
            "filesystem_access": "none"
        }
    })
}

fn capability_contract_json(capability_id: &str, name: &str) -> Value {
    let namespace = component_namespace(capability_id);
    serde_json::json!({
        "kind": "capability_contract",
        "schema_version": "1.0.0",
        "id": capability_id,
        "namespace": namespace,
        "name": name,
        "version": "1.0.0",
        "lifecycle": "active",
        "owner": {
            "team": "local-author",
            "contact": "local-author"
        },
        "summary": format!("Governed capability contract for {capability_id}."),
        "description": format!("Draft Traverse capability contract for the real WASM capability {capability_id}."),
        "inputs": {
            "schema": {
                "type": "object",
                "required": ["input_value"],
                "additionalProperties": false,
                "properties": {
                    "input_value": {
                        "type": "string",
                        "description": "Placeholder input field. Replace with this capability's real input fields."
                    }
                }
            }
        },
        "outputs": {
            "schema": {
                "type": "object",
                "required": ["output_value"],
                "additionalProperties": false,
                "properties": {
                    "output_value": {
                        "type": "string",
                        "description": "Placeholder output field. Replace with this capability's real output fields."
                    }
                }
            }
        },
        "preconditions": [],
        "postconditions": [],
        "side_effects": [
            {
                "kind": "memory_only",
                "description": "No external side effects are declared for this draft capability contract."
            }
        ],
        "emits": [],
        "consumes": [],
        "permissions": [
            {
                "id": capability_id
            }
        ],
        "execution": {
            "binary_format": "wasm",
            "entrypoint": {
                "kind": "wasi-command",
                "command": "run"
            },
            "preferred_targets": ["local"],
            "constraints": {
                "host_api_access": "none",
                "network_access": "forbidden",
                "filesystem_access": "none"
            }
        },
        "policies": [
            {
                "id": "manual-approval-required"
            }
        ],
        "dependencies": [],
        "provenance": {
            "source": "greenfield",
            "author": "traverse-cli",
            "created_at": "capability-scaffold",
            "spec_ref": "100-capability-package-authoring@1.0.0",
            "adr_refs": [],
            "exception_refs": []
        },
        "evidence": [],
        "service_type": "stateless",
        "permitted_targets": ["local"],
        "artifact_type": "native"
    })
}

fn capability_sample_runtime_request_json(capability_id: &str, leaf_name: &str) -> Value {
    serde_json::json!({
        "kind": "runtime_request",
        "schema_version": "1.0.0",
        "request_id": format!("{leaf_name}-scaffold-001"),
        "intent": {
            "capability_id": capability_id,
            "capability_version": "1.0.0"
        },
        "input": {
            "input_value": "example value"
        },
        "lookup": {
            "scope": "prefer_private",
            "allow_ambiguity": false
        },
        "context": {
            "requested_target": "local",
            "caller": "capability-scaffold"
        },
        "governing_spec": "006-runtime-request-execution"
    })
}

/// A `#![no_std]` WASI guest stub matching the no-std guest profile
/// (`091-no-std-wasi-guest-profile`): it imports only
/// `wasi_snapshot_preview1::fd_write` (never `environ_get`) and emits a
/// static placeholder that matches `capability_contract_json`'s output
/// schema. It is an honest placeholder, not fake business logic (`044`
/// QG-004) — the author must implement real input handling and logic here.
/// The template's own source keeps `unsafe` syntax, as any real
/// `#![no_std]` WASI guest must (spec `091-no-std-wasi-guest-profile`
/// FR-004's raw host-import boundary). It lives in a `.rs.template` data
/// file rather than a `.rs` string literal here specifically so
/// `scripts/ci/scoped_unsafe_boundary_check.sh`'s repo-wide `unsafe` grep
/// (scoped to files this crate itself compiles as Rust) does not mistake
/// *generated scaffold text* for unsafe code in `traverse-cli` itself.
fn capability_guest_stub_source() -> String {
    include_str!("../templates/capability_guest_stub.rs.template").to_string()
}

fn capability_build_fixture_script(wasm_name: &str) -> String {
    // Self-contained scaffold script: compile the guest, then write the
    // matching FNV-1a 64 digest into manifest.json so authors never hand-paste it.
    format!(
        r#"#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd "$(dirname "${{BASH_SOURCE[0]}}")" && pwd)"
artifact_dir="$script_dir/artifacts"
artifact_path="$artifact_dir/{wasm_name}"
manifest_path="$script_dir/manifest.json"

mkdir -p "$artifact_dir"

rustup run "$(rustup show active-toolchain | awk '{{print $1}}')" rustc "$script_dir/src/agent.rs" \
  --target wasm32-unknown-unknown --crate-type cdylib -O -C panic=abort -C strip=symbols \
  --remap-path-prefix "$script_dir=/traverse-repo/agent" -o "$artifact_path"

digest="$(python3 - "$artifact_path" <<'PY'
import sys
from pathlib import Path

data = Path(sys.argv[1]).read_bytes()
h = 0xcbf29ce484222325
for b in data:
    h ^= b
    h = (h * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
print(f"fnv1a64:{{h:016x}}")
PY
)"

python3 - "$manifest_path" "$digest" <<'PY'
import json
import sys
from pathlib import Path

manifest = Path(sys.argv[1])
digest = sys.argv[2]
data = json.loads(manifest.read_text())
data.setdefault("binary", {{}})["expected_digest"] = digest
manifest.write_text(json.dumps(data, indent=2) + "\n")
print(f"updated {{manifest}}: binary.expected_digest={{digest}}")
PY

printf 'built %s\n' "$artifact_path"
"#
    )
}

fn register_generated_app_bundle(
    app_id: &str,
    workspace_id: &str,
    manifest_path: &Path,
) -> Result<String, CliError> {
    let manifest_value = read_json_file(manifest_path)?;
    let components_empty = manifest_value
        .get("components")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty);
    let workflows_empty = manifest_value
        .get("workflows")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty);
    if components_empty || workflows_empty {
        return Err(CliError::ValidationFailed(format!(
            "app bundle {app_id} is incomplete: add at least one real component reference and one workflow before registration"
        )));
    }

    let mut apps = ApplicationRegistry::new();
    let mut capabilities = CapabilityRegistry::new();
    let events = EventRegistry::new();
    let mut workflows = WorkflowRegistry::new();
    let outcome = apps
        .register_bundle(
            &mut capabilities,
            &events,
            &mut workflows,
            &ApplicationRegistrationRequest {
                scope: RegistryScope::Private,
                workspace_id: workspace_id.to_string(),
                manifest_path: manifest_path.to_path_buf(),
                registered_at: format!("app:{app_id}@1.0.0"),
                validator_version: env!("CARGO_PKG_VERSION").to_string(),
            },
        )
        .map_err(|failure| {
            CliError::ValidationFailed(render_application_registration_failure(failure.errors))
        })?;

    Ok(format!(
        "registration_status: {:?}\nhttp_status: {}\nworkspace_id: {}",
        outcome.status,
        outcome.status.http_status(),
        outcome.record.workspace_id
    ))
}

fn app_validate(
    manifest_path: &Path,
    workspace_id: Option<&str>,
    json_output: bool,
) -> Result<String, CliError> {
    let base_dir = std::env::current_dir().map_err(|error| {
        CliError::IoError(format!("failed to resolve current directory: {error}"))
    })?;
    app_validate_at(&base_dir, manifest_path, workspace_id, json_output)
}

fn app_validate_at(
    base_dir: &Path,
    manifest_path: &Path,
    workspace_id: Option<&str>,
    json_output: bool,
) -> Result<String, CliError> {
    if !json_output {
        return Err(CliError::UsageError(
            "app validate requires --json for stable setup evidence".to_string(),
        ));
    }

    if let Some(error) = validate_app_manifest_metadata_for_cli(manifest_path)? {
        return render_app_validation_failure(manifest_path, vec![error]);
    }

    let manifest = if let Some(workspace_id) = workspace_id {
        let resolver = SyncedRegistryComponentResolver {
            workspace_root: base_dir,
            workspace_id,
        };
        load_application_bundle_manifest_with_resolver(manifest_path, Some(&resolver))
    } else {
        load_application_bundle_manifest(manifest_path)
    };
    match manifest {
        Ok(manifest) => render_app_validation_success(manifest_path, &manifest),
        Err(failure) => Ok(render_app_validation_failure(
            manifest_path,
            failure
                .errors
                .into_iter()
                .map(AppValidationError::from_manifest_error)
                .collect(),
        )?),
    }
}

fn app_register(
    manifest_path: &Path,
    workspace_id: &str,
    json_output: bool,
) -> Result<String, CliError> {
    let base_dir = std::env::current_dir()
        .map_err(|e| CliError::IoError(format!("failed to resolve current directory: {e}")))?;
    app_register_at(&base_dir, manifest_path, workspace_id, json_output)
}

struct SyncedRegistryComponentResolver<'a> {
    workspace_root: &'a Path,
    workspace_id: &'a str,
}

impl RegistryComponentResolver for SyncedRegistryComponentResolver<'_> {
    fn resolve(
        &self,
        reference: &RegistryReference,
    ) -> Result<ResolvedRegistryComponent, ApplicationManifestFailure> {
        let record = traverse_registry::resolve_synced_public_registry_range(
            self.workspace_root,
            self.workspace_id,
            &reference.namespace,
            &reference.id,
            &reference.version_range,
        )
        .map_err(|failure| {
            registry_resolution_failure(failure.errors.first().map_or_else(
                || "public registry resolution failed".to_string(),
                |error| error.message.clone(),
            ))
        })?;
        let contract_path = cache_registry_asset(
            self.workspace_root,
            &record.contract_url,
            &record.contract_digest,
        )?;
        let contract_text = fs::read_to_string(&contract_path).map_err(|error| {
            registry_resolution_failure(format!("failed to read cached registry contract: {error}"))
        })?;
        let contract = parse_contract(&contract_text).map_err(|failure| {
            registry_resolution_failure(format!(
                "cached registry contract is invalid: {}",
                failure.errors[0].message
            ))
        })?;
        let wasm_binary_path =
            cache_registry_asset(self.workspace_root, &record.artifact_url, &record.digest)?;
        Ok(ResolvedRegistryComponent {
            contract_path,
            contract,
            wasm_binary_path,
            wasm_digest: record.digest,
        })
    }
}

fn registry_resolution_failure(message: String) -> ApplicationManifestFailure {
    ApplicationManifestFailure {
        errors: vec![ApplicationManifestError {
            code: ApplicationManifestErrorCode::RegistryReferenceRequiresResolution,
            path: "$.registry_ref".to_string(),
            message,
        }],
    }
}

fn cache_registry_asset(
    workspace_root: &Path,
    url: &str,
    expected_digest: &str,
) -> Result<PathBuf, ApplicationManifestFailure> {
    if let Some(cache_path) = public_registry_cache_path(workspace_root, expected_digest)
        && cache_path.exists()
    {
        let cached = fs::read(&cache_path).map_err(|error| {
            registry_resolution_failure(format!("failed to read cached registry asset: {error}"))
        })?;
        return cache_verified_public_registry_bytes(workspace_root, expected_digest, &cached)
            .map_err(|failure| registry_resolution_failure(failure.message));
    }
    let output = std::process::Command::new("curl")
        .args(["-fsSL", url])
        .output()
        .map_err(|error| {
            registry_resolution_failure(format!("failed to fetch registry asset: {error}"))
        })?;
    if !output.status.success() {
        return Err(registry_resolution_failure(
            "registry asset download failed".to_string(),
        ));
    }
    cache_verified_public_registry_bytes(workspace_root, expected_digest, &output.stdout)
        .map_err(|failure| registry_resolution_failure(failure.message))
}

fn app_register_at(
    base_dir: &Path,
    manifest_path: &Path,
    workspace_id: &str,
    json_output: bool,
) -> Result<String, CliError> {
    if !json_output {
        return Err(CliError::UsageError(
            "app register requires --json for stable setup evidence".to_string(),
        ));
    }

    if let Some(error) = validate_workspace_id_for_cli(workspace_id) {
        return render_app_registration_failure(manifest_path, workspace_id, vec![error], None);
    }

    if let Some(error) = validate_app_manifest_metadata_for_cli(manifest_path)? {
        return render_app_registration_failure(manifest_path, workspace_id, vec![error], None);
    }

    let resolver = SyncedRegistryComponentResolver {
        workspace_root: base_dir,
        workspace_id,
    };
    let manifest =
        match load_application_bundle_manifest_with_resolver(manifest_path, Some(&resolver)) {
            Ok(manifest) => manifest,
            Err(failure) => {
                return render_app_registration_failure(
                    manifest_path,
                    workspace_id,
                    failure
                        .errors
                        .into_iter()
                        .map(AppValidationError::from_manifest_error)
                        .collect(),
                    None,
                );
            }
        };

    let state_path =
        app_registration_state_path(base_dir, workspace_id, &manifest.app_id, &manifest.version);
    let mut state = match render_app_registration_state(manifest_path, workspace_id, &manifest) {
        Ok(state) => state,
        Err(error) => {
            return render_app_registration_failure(manifest_path, workspace_id, vec![error], None);
        }
    };
    let fingerprint = state["registration_fingerprint"].clone();
    let status = match read_existing_registration_fingerprint(&state_path)? {
        Some(existing) if existing == fingerprint => "already_registered",
        Some(_) => {
            return render_app_registration_failure(
                manifest_path,
                workspace_id,
                vec![AppValidationError {
                    code: "registration_conflict".to_string(),
                    path: state_path.display().to_string(),
                    message: "workspace already contains different registration state for this app version".to_string(),
                }],
                Some(&state_path),
            );
        }
        None => "registered",
    };

    state["status"] = Value::String(status.to_string());
    if status == "registered"
        && let Err(error) = write_registration_state_atomically(&state_path, &state)
    {
        return render_app_registration_failure(
            manifest_path,
            workspace_id,
            vec![error],
            Some(&state_path),
        );
    }

    serde_json::to_string_pretty(&state)
        .map_err(|e| CliError::IoError(format!("failed to serialize app registration: {e}")))
}

#[derive(Debug, Deserialize)]
struct HostActivationInput {
    #[serde(default)]
    connectors: Vec<HostConnectorActivationInput>,
    #[serde(default)]
    artifacts: Vec<HostArtifactActivationInput>,
}

#[derive(Debug, Deserialize)]
struct HostConnectorActivationInput {
    connector_id: String,
    installed_version: String,
    placement_target: traverse_contracts::ExecutionTarget,
    config: Value,
}

/// Host-local executable candidates for one required capability contract.
/// This input deliberately carries no artifact bytes or private configuration.
#[derive(Debug, Deserialize)]
struct HostArtifactActivationInput {
    contract_reference: String,
    placement_target: traverse_contracts::ExecutionTarget,
    #[serde(default)]
    config_refs: Vec<String>,
    #[serde(default)]
    candidates: Vec<HostExecutableArtifactCandidate>,
}

#[derive(Debug, Deserialize)]
struct HostExecutableArtifactCandidate {
    package_id: String,
    package_version: String,
    digest: String,
    abi: String,
    lifecycle: Lifecycle,
    placement: Vec<traverse_contracts::ExecutionTarget>,
    execution_constraints: String,
}

#[allow(clippy::too_many_lines)]
fn app_activate(
    manifest_path: &Path,
    workspace_id: &str,
    host_activation_path: &Path,
    json_output: bool,
) -> Result<String, CliError> {
    let base_dir = std::env::current_dir().map_err(|error| {
        CliError::IoError(format!("failed to resolve current directory: {error}"))
    })?;
    app_activate_at(
        &base_dir,
        manifest_path,
        workspace_id,
        host_activation_path,
        json_output,
    )
}

#[allow(clippy::too_many_lines)]
fn app_activate_at(
    state_root: &Path,
    manifest_path: &Path,
    workspace_id: &str,
    host_activation_path: &Path,
    json_output: bool,
) -> Result<String, CliError> {
    if !json_output {
        return Err(CliError::UsageError(
            "app activate requires --json for stable activation evidence".to_string(),
        ));
    }
    if let Some(error) = validate_workspace_id_for_cli(workspace_id) {
        return render_app_activation_failure(manifest_path, workspace_id, vec![error]);
    }

    let manifest = match load_application_bundle_manifest(manifest_path) {
        Ok(manifest) => manifest,
        Err(failure) => {
            return render_app_activation_failure(
                manifest_path,
                workspace_id,
                failure
                    .errors
                    .into_iter()
                    .map(AppValidationError::from_manifest_error)
                    .collect(),
            );
        }
    };
    let host_input: HostActivationInput =
        read_json_file(host_activation_path).and_then(|value| {
            serde_json::from_value(value).map_err(|error| {
                CliError::ValidationFailed(format!(
                    "host_activation_invalid: {}: {error}",
                    host_activation_path.display()
                ))
            })
        })?;

    let mut seen = std::collections::BTreeSet::new();
    let mut errors = Vec::new();
    for input in &host_input.connectors {
        if !seen.insert(input.connector_id.clone()) {
            errors.push(AppValidationError {
                code: "connector_activation_duplicate".to_string(),
                path: format!("$.connectors[{}]", input.connector_id),
                message: format!("duplicate host activation input for {}", input.connector_id),
            });
        }
    }
    for binding in &manifest.connector_bindings {
        if !seen.contains(&binding.connector_id) {
            errors.push(AppValidationError {
                code: "connector_activation_missing".to_string(),
                path: format!("$.connector_bindings[{}]", binding.connector_id),
                message: format!("host activation input is missing {}", binding.connector_id),
            });
        }
    }
    for input in &host_input.connectors {
        if !manifest
            .connector_bindings
            .iter()
            .any(|binding| binding.connector_id == input.connector_id)
        {
            errors.push(AppValidationError {
                code: "connector_activation_undeclared".to_string(),
                path: format!("$.connectors[{}]", input.connector_id),
                message: format!(
                    "host activation input declares unbound connector {}",
                    input.connector_id
                ),
            });
        }
    }
    if !errors.is_empty() {
        return render_app_activation_failure(manifest_path, workspace_id, errors);
    }

    let mut registry = CapabilityRegistry::new();
    for connector in reference_connector_contracts() {
        registry
            .register_connector(ConnectorRegistration {
                scope: RegistryScope::Public,
                contract_path: format!(
                    "contracts/connectors/{}/connector_contract.json",
                    connector.connector_id
                ),
                contract: connector,
                registered_at: "host-activation".to_string(),
                governing_spec: "039-connector-plugin-architecture".to_string(),
                validator_version: env!("CARGO_PKG_VERSION").to_string(),
            })
            .map_err(|failure| CliError::ValidationFailed(render_registry_failure(failure)))?;
    }

    let mut evidence = Vec::new();
    for binding in &manifest.connector_bindings {
        let input = host_input
            .connectors
            .iter()
            .find(|input| input.connector_id == binding.connector_id)
            .ok_or_else(|| {
                CliError::ValidationFailed("connector activation input disappeared".to_string())
            })?;
        match validate_connector_activation(
            &registry,
            LookupScope::PreferPrivate,
            &manifest.connector_bindings,
            &ConnectorActivationRequest {
                connector_id: input.connector_id.clone(),
                installed: InstalledConnector {
                    connector_id: input.connector_id.clone(),
                    version: input.installed_version.clone(),
                },
                placement_target: input.placement_target.clone(),
                host_config: input.config.clone(),
            },
        ) {
            Ok(record) => evidence.push(serde_json::json!({
                "connector_id": record.connector_id,
                "config_ref": binding.config_ref,
                "resolved_version": record.resolved_version,
                "placement_target": record.placement_target,
                "config_keys_present": record.config_keys_present,
                "evidence_digest": record.evidence_digest,
            })),
            Err(failure) => {
                let errors = failure
                    .errors
                    .into_iter()
                    .map(|error| AppValidationError {
                        code: debug_enum_to_snake_case(&format!("{:?}", error.code)),
                        path: format!("$.connector_bindings[{}]", error.connector_id),
                        message: error.message,
                    })
                    .collect();
                return render_app_activation_failure(manifest_path, workspace_id, errors);
            }
        }
    }
    evidence.sort_by(|left, right| {
        left["connector_id"]
            .as_str()
            .cmp(&right["connector_id"].as_str())
    });
    let mut artifact_evidence = Vec::new();
    let mut required_contracts = std::collections::BTreeSet::new();
    for component in &manifest.components {
        required_contracts.insert(format!(
            "{}@{}",
            component.manifest.capability_id, component.manifest.capability_version
        ));
    }
    let mut artifact_input_ids = std::collections::BTreeSet::new();
    for input in &host_input.artifacts {
        if !artifact_input_ids.insert(input.contract_reference.clone()) {
            errors.push(AppValidationError {
                code: "executable_artifact_duplicate".to_string(),
                path: format!("$.artifacts[{}]", input.contract_reference),
                message: format!(
                    "duplicate host activation input for {}",
                    input.contract_reference
                ),
            });
        }
        if !required_contracts.contains(&input.contract_reference) {
            errors.push(AppValidationError {
                code: "executable_artifact_undeclared".to_string(),
                path: format!("$.artifacts[{}]", input.contract_reference),
                message: format!(
                    "host activation input declares an artifact for undeclared contract {}",
                    input.contract_reference
                ),
            });
        }
    }
    for contract_reference in &required_contracts {
        if !artifact_input_ids.contains(contract_reference) {
            errors.push(AppValidationError {
                code: "executable_artifact_unavailable".to_string(),
                path: format!("$.components[{contract_reference}]"),
                message: format!(
                    "host activation input is missing executable artifacts for {contract_reference}"
                ),
            });
        }
    }
    if !errors.is_empty() {
        return render_app_activation_failure(manifest_path, workspace_id, errors);
    }

    for contract_reference in required_contracts {
        let Some(input) = host_input
            .artifacts
            .iter()
            .find(|input| input.contract_reference == contract_reference)
        else {
            return render_app_activation_failure(
                manifest_path,
                workspace_id,
                vec![AppValidationError {
                    code: "executable_artifact_unavailable".to_string(),
                    path: format!("$.components[{contract_reference}]"),
                    message: format!(
                        "host activation input is missing executable artifacts for {contract_reference}"
                    ),
                }],
            );
        };
        let pin = manifest.components.iter().find_map(|component| {
            (format!(
                "{}@{}",
                component.manifest.capability_id, component.manifest.capability_version
            ) == contract_reference)
                .then(|| component.manifest.executable_pin.clone())
                .flatten()
        });
        let candidates = input
            .candidates
            .iter()
            .map(|candidate| ExecutableArtifactCandidate {
                package_id: candidate.package_id.clone(),
                package_version: candidate.package_version.clone(),
                contract_reference: contract_reference.clone(),
                digest: candidate.digest.clone(),
                abi: candidate.abi.clone(),
                lifecycle: candidate.lifecycle.clone(),
                placement: candidate.placement.clone(),
                execution_constraints: candidate.execution_constraints.clone(),
            })
            .collect::<Vec<_>>();
        let resolution = match resolve_executable_artifact(
            &ArtifactResolutionRequest {
                contract_reference: contract_reference.clone(),
                placement_target: input.placement_target.clone(),
                config_refs: input.config_refs.clone(),
                pin,
            },
            &candidates,
        ) {
            Ok(resolution) => resolution,
            Err(failure) => {
                return render_app_activation_failure(
                    manifest_path,
                    workspace_id,
                    failure
                        .errors
                        .into_iter()
                        .map(|error| AppValidationError {
                            code: debug_enum_to_snake_case(&format!("{:?}", error.code)),
                            path: format!("$.artifacts[{contract_reference}]"),
                            message: error.message,
                        })
                        .collect(),
                );
            }
        };
        artifact_evidence.push(serde_json::json!({
            "contract_reference": resolution.contract_reference,
            "selected_package_id": resolution.selected_package_id,
            "selected_package_version": resolution.selected_package_version,
            "selected_digest": resolution.selected_digest,
            "selected_lifecycle": resolution.selected_lifecycle,
            "selected_abi": resolution.selected_abi,
            "selected_placement": resolution.selected_placement,
            "selected_execution_constraints": resolution.selected_execution_constraints,
            "resolver_version": resolution.resolver_version,
            "eligibility_decisions": resolution.eligibility_decisions,
            "config_refs": input.config_refs,
            "evidence_digest": resolution.evidence_digest,
        }));
    }
    let activation = serde_json::json!({
        "status": "activated",
        "workspace_id": workspace_id,
        "app_id": manifest.app_id,
        "app_version": manifest.version,
        "manifest_path": manifest_path.display().to_string(),
        "connectors": evidence,
        "artifacts": artifact_evidence,
        "governing_specs": ["039-connector-plugin-architecture", "103-application-connector-binding", "106-activation-artifact-resolution"],
    });
    let state_path = app_activation_state_path(
        state_root,
        workspace_id,
        &manifest.app_id,
        &manifest.version,
    );
    write_registration_state_atomically(&state_path, &activation)
        .map_err(|error| CliError::IoError(format!("{}: {}", error.code, error.message)))?;
    serde_json::to_string_pretty(&activation).map_err(|error| {
        CliError::IoError(format!("failed to serialize activation evidence: {error}"))
    })
}

fn render_app_activation_failure(
    manifest_path: &Path,
    workspace_id: &str,
    errors: Vec<AppValidationError>,
) -> Result<String, CliError> {
    let errors = errors
        .into_iter()
        .map(|error| {
            serde_json::json!({
                "code": error.code,
                "path": error.path,
                "message": error.message,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&serde_json::json!({
        "status": "activation_failed",
        "workspace_id": workspace_id,
        "manifest_path": manifest_path.display().to_string(),
        "errors": errors,
    }))
    .map_err(|error| CliError::IoError(format!("failed to serialize activation failure: {error}")))
}

const DEFAULT_PUBLIC_REGISTRY_SOURCE: &str = "traverse-framework/registry";

#[derive(Debug, Clone)]
struct FetchedRegistryIndex {
    source_repo: String,
    release_tag: String,
    index: PublicRegistryIndex,
}

trait RegistryIndexFetcher {
    fn fetch_latest_index(&self) -> Result<FetchedRegistryIndex, RegistrySyncError>;
}

#[derive(Debug, Clone)]
struct CurlGitHubRegistryIndexFetcher {
    source_repo: String,
    /// Bearer token for a private source repo's Releases API. `None` for the
    /// default public source, which is unauthenticated.
    token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegistrySyncError {
    code: &'static str,
    message: String,
}

impl RegistryIndexFetcher for CurlGitHubRegistryIndexFetcher {
    fn fetch_latest_index(&self) -> Result<FetchedRegistryIndex, RegistrySyncError> {
        let releases_url = format!(
            "https://api.github.com/repos/{}/releases?per_page=100",
            self.source_repo
        );
        let releases = curl_text(&releases_url, self.token.as_deref())?;
        let releases_json: Value = serde_json::from_str(&releases).map_err(|error| {
            RegistrySyncError::new(
                "registry_release_parse_failed",
                format!("failed to parse GitHub Releases response: {error}"),
            )
        })?;
        let (release_tag, index_asset_url) = latest_index_release_asset(&releases_json)?;
        let index_json = curl_text(&index_asset_url, self.token.as_deref())?;
        let index: PublicRegistryIndex = serde_json::from_str(&index_json).map_err(|error| {
            RegistrySyncError::new(
                "registry_index_parse_failed",
                format!("failed to parse registry index.json: {error}"),
            )
        })?;

        Ok(FetchedRegistryIndex {
            source_repo: self.source_repo.clone(),
            release_tag,
            index,
        })
    }
}

impl RegistrySyncError {
    fn new(code: &'static str, message: String) -> Self {
        Self { code, message }
    }
}

/// Resolves the effective registry source repo: the explicit override when
/// supplied, otherwise the default public registry.
fn registry_sync_default_or_override(source_repo_override: Option<String>) -> String {
    source_repo_override.unwrap_or_else(|| DEFAULT_PUBLIC_REGISTRY_SOURCE.to_string())
}

fn registry_sync(
    workspace_id: &str,
    json_output: bool,
    source_repo_override: Option<String>,
) -> Result<String, CliError> {
    let base_dir = std::env::current_dir()
        .map_err(|e| CliError::IoError(format!("failed to resolve current directory: {e}")))?;
    let is_default_source = source_repo_override.is_none();
    let source_repo = registry_sync_default_or_override(source_repo_override);
    // Only read/attach the token for a non-default source. The default
    // public registry stays unauthenticated -- wire-identical to today --
    // regardless of whether TRAVERSE_REGISTRY_TOKEN happens to be set in
    // the caller's environment for an unrelated reason.
    let token = if is_default_source {
        None
    } else {
        std::env::var("TRAVERSE_REGISTRY_TOKEN").ok()
    };
    registry_sync_at(
        &base_dir,
        workspace_id,
        json_output,
        &CurlGitHubRegistryIndexFetcher { source_repo, token },
    )
}

fn registry_sync_at(
    base_dir: &Path,
    workspace_id: &str,
    json_output: bool,
    fetcher: &dyn RegistryIndexFetcher,
) -> Result<String, CliError> {
    if !json_output {
        return Err(CliError::UsageError(
            "registry sync requires --json for stable sync evidence".to_string(),
        ));
    }
    if let Some(error) = validate_workspace_id_for_cli(workspace_id) {
        return registry_sync_failure_json(
            workspace_id,
            "registry_sync_invalid_workspace",
            &error.message,
        );
    }

    let remote_index = fetcher
        .fetch_latest_index()
        .map_err(|error| CliError::IoError(format!("{}: {}", error.code, error.message)))?;
    let synced_at = current_unix_timestamp_string()?;
    let state = write_synced_public_registry_state(
        base_dir,
        workspace_id,
        &remote_index.source_repo,
        &remote_index.release_tag,
        &synced_at,
        remote_index.index,
    )
    .map_err(|failure| {
        CliError::ValidationFailed(
            failure
                .errors
                .iter()
                .map(|error| error.message.clone())
                .collect::<Vec<_>>()
                .join("; "),
        )
    })?;
    let state_path = traverse_registry::synced_public_registry_state_path(base_dir, workspace_id);

    serde_json::to_string_pretty(&serde_json::json!({
        "status": "synced",
        "source": state.source_repo,
        "release_tag": state.release_tag,
        "index_version": state.index_version,
        "record_count": state.record_count,
        "workspace": state.workspace_id,
        "synced_at": state.synced_at,
        "validation_status": state.validation_status,
        "state_path": state_path.display().to_string()
    }))
    .map_err(|error| {
        CliError::IoError(format!("failed to serialize registry sync output: {error}"))
    })
}

fn registry_sync_failure_json(
    workspace_id: &str,
    code: &str,
    message: &str,
) -> Result<String, CliError> {
    serde_json::to_string_pretty(&serde_json::json!({
        "status": "failed",
        "workspace": workspace_id,
        "errors": [{
            "code": code,
            "message": message,
            "severity": "error"
        }]
    }))
    .map_err(|error| {
        CliError::IoError(format!(
            "failed to serialize registry sync failure: {error}"
        ))
    })
}

fn registry_list(
    workspace_id: &str,
    namespace: Option<&str>,
    id_prefix: Option<&str>,
    json_output: bool,
) -> Result<String, CliError> {
    registry_discover(workspace_id, namespace, id_prefix, None, json_output)
}

fn registry_search(
    query: &str,
    workspace_id: &str,
    namespace: Option<&str>,
    json_output: bool,
) -> Result<String, CliError> {
    registry_discover(workspace_id, namespace, None, Some(query), json_output)
}

fn registry_discover(
    workspace_id: &str,
    namespace: Option<&str>,
    id_prefix: Option<&str>,
    query: Option<&str>,
    json_output: bool,
) -> Result<String, CliError> {
    let base_dir = env::current_dir().map_err(|error| {
        CliError::IoError(format!("failed to resolve current directory: {error}"))
    })?;
    let state = traverse_registry::load_synced_public_registry_state(&base_dir, workspace_id)
        .map_err(|failure| registry_discovery_state_error(&failure))?;
    let query = query.map(str::to_lowercase);
    let mut records = state
        .capabilities
        .into_iter()
        .filter(|record| namespace.is_none_or(|value| record.namespace == value))
        .filter(|record| id_prefix.is_none_or(|value| record.id.starts_with(value)))
        .filter(|record| {
            query.as_ref().is_none_or(|value| {
                record.namespace.to_lowercase().contains(value)
                    || record.id.to_lowercase().contains(value)
            })
        })
        .collect::<Vec<_>>();
    records.sort_by(registry_record_order);

    if json_output {
        return serde_json::to_string_pretty(&serde_json::json!({
            "status": "ok",
            "workspace": state.workspace_id,
            "source_release": state.release_tag,
            "index_version": state.index_version,
            "source_commit": state.source_commit,
            "synced_at": state.synced_at,
            "stale": false,
            "records": records.into_iter().map(|record| serde_json::json!({
                "namespace": record.namespace,
                "id": record.id,
                "version": record.version,
                "digest": record.digest,
                "yanked": false,
                "deprecated": record.deprecated,
            })).collect::<Vec<_>>(),
        }))
        .map_err(|error| {
            CliError::IoError(format!(
                "failed to serialize registry discovery output: {error}"
            ))
        });
    }

    let mut output = String::from("NAMESPACE\tID\tVERSION\tDEPRECATED\n");
    for record in records {
        writeln!(
            output,
            "{}\t{}\t{}\t{}",
            record.namespace, record.id, record.version, record.deprecated
        )
        .map_err(|error| {
            CliError::IoError(format!(
                "failed to render registry discovery output: {error}"
            ))
        })?;
    }
    Ok(output)
}

fn registry_discovery_state_error(
    failure: &traverse_registry::PublicRegistryStateFailure,
) -> CliError {
    let message = failure
        .errors
        .iter()
        .map(|error| error.message.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    let code = if failure.errors.iter().any(|error| {
        matches!(
            error.code,
            traverse_registry::PublicRegistryStateErrorCode::MissingSyncedState
        )
    }) {
        "registry_sync_missing"
    } else {
        "registry_sync_invalid"
    };
    CliError::ValidationFailed(format!("{code}: {message}"))
}

fn registry_record_order(
    left: &PublicRegistryCapabilityRecord,
    right: &PublicRegistryCapabilityRecord,
) -> std::cmp::Ordering {
    left.namespace
        .cmp(&right.namespace)
        .then_with(|| left.id.cmp(&right.id))
        .then_with(|| registry_version_order(&left.version, &right.version))
}

fn registry_version_order(left: &str, right: &str) -> std::cmp::Ordering {
    match (Version::parse(left), Version::parse(right)) {
        (Ok(left), Ok(right)) => right.cmp(&left),
        _ => right.cmp(left),
    }
}

const CAPABILITY_PUBLISH_GOVERNING_SPEC: &str = "056-capability-publish";
const CAPABILITY_PUBLISH_VALIDATOR_VERSION: &str = "traverse-cli capability publish";
const DEFAULT_REGISTRY_REPO: &str = "traverse-framework/registry";

#[derive(Debug, Clone)]
struct CapabilityPublishRequest {
    contract_path: PathBuf,
    artifact_path: PathBuf,
    registry_repo_path: PathBuf,
    /// Overrides `DEFAULT_REGISTRY_REPO` as the `gh pr create --repo` target.
    /// `None` publishes against traverse-framework/registry, unchanged.
    registry_repo_remote: Option<String>,
    json_output: bool,
    dry_run: bool,
}

#[derive(Debug, Clone)]
struct CapabilityPublishPlan {
    namespace: String,
    capability_id: String,
    version: String,
    artifact_digest: String,
    artifact_asset_name: String,
    artifact_release_tag: String,
    artifact_url: String,
    registry_relative_path: PathBuf,
    registry_path: PathBuf,
    branch: String,
    title: String,
    contract_json: String,
}

#[derive(Debug, Clone)]
struct PublishCommandOutput {
    stdout: String,
}

trait PublishProcessRunner {
    fn run(
        &self,
        cwd: &Path,
        program: &str,
        args: &[String],
    ) -> Result<PublishCommandOutput, String>;
}

struct RealPublishProcessRunner;

impl PublishProcessRunner for RealPublishProcessRunner {
    fn run(
        &self,
        cwd: &Path,
        program: &str,
        args: &[String],
    ) -> Result<PublishCommandOutput, String> {
        let output = std::process::Command::new(program)
            .args(args)
            .current_dir(cwd)
            .output()
            .map_err(|error| format!("failed to execute {program}: {error}"))?;
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !output.status.success() {
            let detail = if stderr.is_empty() {
                format!("{program} exited with status {}", output.status)
            } else {
                stderr.clone()
            };
            return Err(detail);
        }
        Ok(PublishCommandOutput { stdout })
    }
}

fn capability_publish(
    contract_path: &Path,
    artifact_path: &Path,
    registry_repo_path: &Path,
    registry_repo_remote: Option<String>,
    json_output: bool,
    dry_run: bool,
) -> Result<String, CliError> {
    capability_publish_at(
        &CapabilityPublishRequest {
            contract_path: contract_path.to_path_buf(),
            artifact_path: artifact_path.to_path_buf(),
            registry_repo_path: registry_repo_path.to_path_buf(),
            registry_repo_remote,
            json_output,
            dry_run,
        },
        &RealPublishProcessRunner,
    )
}

fn capability_publish_at(
    request: &CapabilityPublishRequest,
    runner: &dyn PublishProcessRunner,
) -> Result<String, CliError> {
    if !request.json_output {
        return Err(CliError::UsageError(
            "capability publish requires --json for stable publish evidence".to_string(),
        ));
    }

    let registry_repo = request
        .registry_repo_remote
        .as_deref()
        .unwrap_or(DEFAULT_REGISTRY_REPO);

    let plan = match capability_publish_plan(request, registry_repo) {
        Ok(plan) => plan,
        Err((code, message)) => {
            return capability_publish_failure_json(
                code,
                &message,
                None,
                None,
                None,
                registry_repo,
            );
        }
    };

    if plan.registry_path.exists() {
        return capability_publish_failure_json(
            "capability_publish_immutable_conflict",
            "target registry path already exists; published versions are immutable",
            Some(&plan),
            None,
            None,
            registry_repo,
        );
    }

    if request.dry_run {
        return capability_publish_success_json("dry_run", &plan, None, registry_repo);
    }

    if let Err(error) = ensure_clean_registry_checkout(&request.registry_repo_path, runner) {
        return capability_publish_failure_json(
            "capability_publish_registry_dirty",
            &error,
            Some(&plan),
            None,
            None,
            registry_repo,
        );
    }

    match prepare_and_open_registry_pr(request, &plan, runner, registry_repo) {
        Ok(url) => capability_publish_success_json("pr_opened", &plan, Some(&url), registry_repo),
        Err((code, message, partial_state)) => capability_publish_failure_json(
            code,
            &message,
            Some(&plan),
            partial_state.as_deref(),
            None,
            registry_repo,
        ),
    }
}

fn capability_publish_plan(
    request: &CapabilityPublishRequest,
    registry_repo: &str,
) -> Result<CapabilityPublishPlan, (&'static str, String)> {
    if !request.registry_repo_path.is_dir() {
        return Err((
            "capability_publish_registry_missing",
            format!(
                "registry repo path does not exist or is not a directory: {}",
                request.registry_repo_path.display()
            ),
        ));
    }

    let contract_text = fs::read_to_string(&request.contract_path).map_err(|error| {
        (
            "capability_publish_contract_read_failed",
            format!("failed to read capability contract: {error}"),
        )
    })?;
    reject_private_contract_scope(&contract_text)?;
    // Spec 102: fail closed on schema ⊆ use_cases before normalization.
    enforce_contract_surface_coverage(&contract_text)?;
    // Fail fast on persona_ref gaps before opening a registry PR.
    enforce_persona_refs_resolve(&contract_text, &request.registry_repo_path)?;

    let raw_contract_value: Value = serde_json::from_str(&contract_text).map_err(|error| {
        (
            "capability_publish_contract_parse_failed",
            format!("failed to parse capability contract JSON: {error}"),
        )
    })?;
    let contract = parse_contract(&contract_text).map_err(|failure| {
        (
            "capability_publish_contract_parse_failed",
            render_validation_failure("capability contract", &request.contract_path, failure),
        )
    })?;
    let validation = validate_contract(
        contract,
        &ValidationContext {
            governing_spec: CAPABILITY_PUBLISH_GOVERNING_SPEC,
            validator_version: CAPABILITY_PUBLISH_VALIDATOR_VERSION,
            existing_published: None,
        },
    )
    .map_err(|failure| {
        (
            "capability_publish_contract_validation_failed",
            render_validation_failure("capability contract", &request.contract_path, failure),
        )
    })?;

    let normalized = validation.normalized;
    validate_registry_path_segment(&normalized.namespace, "namespace")?;
    validate_registry_path_segment(&normalized.id, "capability id")?;
    validate_registry_path_segment(&normalized.version, "version")?;
    let artifact_digest = publish_file_sha256_digest(&request.artifact_path)?;
    let artifact_asset_name = publish_artifact_asset_name(&request.artifact_path)?;
    let artifact_release_tag = format!("artifacts/{}-{}", normalized.id, normalized.version);
    let artifact_url = format!(
        "https://github.com/{registry_repo}/releases/download/{artifact_release_tag}/{artifact_asset_name}"
    );

    let registry_relative_path = PathBuf::from("capabilities")
        .join(&normalized.namespace)
        .join(&normalized.id)
        .join(&normalized.version)
        .join("contract.json");
    let registry_path = request.registry_repo_path.join(&registry_relative_path);
    let branch = format!(
        "publish/{}-{}",
        sanitize_branch_component(&normalized.id),
        sanitize_branch_component(&normalized.version)
    );
    let title = format!("Publish {} {}", normalized.id, normalized.version);
    let mut contract_value = serde_json::to_value(&normalized).map_err(|error| {
        (
            "capability_publish_contract_serialize_failed",
            format!("failed to serialize normalized capability contract: {error}"),
        )
    })?;
    merge_author_fields_into_publish_contract(&mut contract_value, &raw_contract_value);
    contract_value["artifact"] = serde_json::json!({
        "digest": artifact_digest,
        "url": artifact_url,
    });
    let contract_json = serde_json::to_string_pretty(&contract_value).map_err(|error| {
        (
            "capability_publish_contract_serialize_failed",
            format!("failed to serialize capability contract with artifact metadata: {error}"),
        )
    })?;

    Ok(CapabilityPublishPlan {
        namespace: normalized.namespace,
        capability_id: normalized.id,
        version: normalized.version,
        artifact_digest,
        artifact_asset_name,
        artifact_release_tag,
        artifact_url,
        registry_relative_path,
        registry_path,
        branch,
        title,
        contract_json,
    })
}

/// Spec 102 FR-005: preserve author `use_cases` and `evidence` into registry-bound JSON.
/// `validate_contract` clears evidence on normalize; merge both from the raw author JSON.
fn merge_author_fields_into_publish_contract(
    contract_value: &mut Value,
    raw_contract_value: &Value,
) {
    if let Some(use_cases) = raw_contract_value.get("use_cases") {
        contract_value["use_cases"] = use_cases.clone();
    }
    if let Some(evidence) = raw_contract_value.get("evidence") {
        contract_value["evidence"] = evidence.clone();
    }
}

fn reject_private_contract_scope(contract_text: &str) -> Result<(), (&'static str, String)> {
    let value: Value = serde_json::from_str(contract_text).map_err(|error| {
        (
            "capability_publish_contract_parse_failed",
            format!("failed to parse capability contract JSON: {error}"),
        )
    })?;
    if value.get("scope").and_then(Value::as_str) == Some("private") {
        return Err((
            "capability_publish_private_scope",
            "private-scoped capability content cannot be published to the public registry"
                .to_string(),
        ));
    }
    Ok(())
}

/// Spec `102-contract-surface-coverage` (Decision 58): fail closed when use_cases
/// do not cover declared input enums, required props, or output reason_code/status enums.
fn enforce_contract_surface_coverage(contract_text: &str) -> Result<(), (&'static str, String)> {
    let value: Value = serde_json::from_str(contract_text).map_err(|error| {
        (
            "capability_publish_contract_parse_failed",
            format!("failed to parse capability contract JSON: {error}"),
        )
    })?;
    match surface_coverage_gap_messages(&value) {
        Ok(gaps) if gaps.is_empty() => Ok(()),
        Ok(gaps) => Err((
            "capability_publish_surface_coverage_failed",
            format!(
                "{}. (spec 102-contract-surface-coverage Decision 58 / FR-001–FR-004)",
                gaps.join("; ")
            ),
        )),
        Err(message) => Err(("capability_publish_surface_coverage_failed", message)),
    }
}

fn surface_coverage_gap_messages(contract: &Value) -> Result<Vec<String>, String> {
    let use_cases = contract
        .get("use_cases")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if use_cases.is_empty() {
        return Ok(vec![
            "use_cases is missing or empty; every capability contract must declare at least one use case (FR-004)"
                .to_string(),
        ]);
    }

    let mut gaps = Vec::new();

    let uncovered_enums = uncovered_input_schema_enum_values(contract, &use_cases)?;
    if !uncovered_enums.is_empty() {
        gaps.push(format!(
            "inputs.schema string enum values lack covering use_cases[].input_example at the same path: {}",
            uncovered_enums.join(", ")
        ));
    }

    let uncovered_required = uncovered_required_input_properties(contract, &use_cases);
    if !uncovered_required.is_empty() {
        gaps.push(format!(
            "inputs.schema.required properties missing from every use_cases[].input_example: {}",
            uncovered_required.join(", ")
        ));
    }

    let uncovered_outputs = uncovered_output_enum_values(contract, &use_cases)?;
    if !uncovered_outputs.is_empty() {
        gaps.push(format!(
            "outputs.schema enum values lack covering use_cases[].output_example: {}",
            uncovered_outputs.join(", ")
        ));
    }

    Ok(gaps)
}

/// Legacy helper retained for action-focused unit tests: uncovered `action` enum values only.
#[cfg(test)]
fn uncovered_action_enum_values(contract: &Value) -> Result<Vec<String>, String> {
    let use_cases = contract
        .get("use_cases")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let all = uncovered_input_schema_enum_values(contract, &use_cases)?;
    Ok(all
        .into_iter()
        .filter_map(|entry| match entry.split_once('=') {
            Some(("action", value)) => Some(value.to_string()),
            _ => None,
        })
        .collect())
}

fn uncovered_input_schema_enum_values(
    contract: &Value,
    use_cases: &[Value],
) -> Result<Vec<String>, String> {
    let Some(schema) = contract.pointer("/inputs/schema") else {
        return Ok(Vec::new());
    };
    let mut declared: Vec<(String, String)> = Vec::new();
    collect_string_enums_under_properties(schema, "", &mut declared)?;
    let mut uncovered = Vec::new();
    for (path, enum_value) in declared {
        let covered = use_cases.iter().any(|use_case| {
            use_case
                .get("input_example")
                .is_some_and(|example| example_covers_path_string(example, &path, &enum_value))
        });
        if !covered {
            uncovered.push(format!("{path}={enum_value}"));
        }
    }
    Ok(uncovered)
}

fn uncovered_required_input_properties(contract: &Value, use_cases: &[Value]) -> Vec<String> {
    let Some(required) = contract
        .pointer("/inputs/schema/required")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut uncovered = Vec::new();
    for item in required {
        let Some(name) = item.as_str() else {
            continue;
        };
        let covered = use_cases.iter().any(|use_case| {
            use_case
                .get("input_example")
                .and_then(Value::as_object)
                .is_some_and(|obj| obj.contains_key(name))
        });
        if !covered {
            uncovered.push(name.to_string());
        }
    }
    uncovered
}

fn uncovered_output_enum_values(
    contract: &Value,
    use_cases: &[Value],
) -> Result<Vec<String>, String> {
    let mut uncovered = Vec::new();
    for field in ["reason_code", "status"] {
        let Some(field_schema) = contract.pointer(&format!("/outputs/schema/properties/{field}"))
        else {
            continue;
        };
        let Some(enum_values) = field_schema.get("enum").and_then(Value::as_array) else {
            continue;
        };
        for value in enum_values {
            let Some(as_str) = value.as_str() else {
                return Err(format!(
                    "outputs.schema.properties.{field}.enum must contain only strings (spec 102 FR-003)"
                ));
            };
            let covered = use_cases.iter().any(|use_case| {
                use_case
                    .pointer(&format!("/output_example/{field}"))
                    .and_then(Value::as_str)
                    == Some(as_str)
            });
            if !covered {
                uncovered.push(format!("{field}={as_str}"));
            }
        }
    }
    Ok(uncovered)
}

/// Walk `properties` recursively (and array `items`), collecting string enums.
/// Skips schemas under `additionalProperties`.
fn collect_string_enums_under_properties(
    schema: &Value,
    path_prefix: &str,
    out: &mut Vec<(String, String)>,
) -> Result<(), String> {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return Ok(());
    };
    for (name, child) in properties {
        let path = if path_prefix.is_empty() {
            name.clone()
        } else {
            format!("{path_prefix}.{name}")
        };
        if let Some(enum_values) = child.get("enum").and_then(Value::as_array) {
            let mut strings = Vec::new();
            for value in enum_values {
                let Some(as_str) = value.as_str() else {
                    return Err(format!(
                        "inputs.schema property '{path}' enum must contain only strings (spec 102 FR-001)"
                    ));
                };
                strings.push(as_str.to_string());
            }
            if !strings.is_empty() {
                for value in strings {
                    out.push((path.clone(), value));
                }
            }
        }
        collect_string_enums_under_properties(child, &path, out)?;
        if let Some(items) = child.get("items") {
            collect_string_enums_under_properties(items, &path, out)?;
        }
    }
    Ok(())
}

fn example_covers_path_string(example: &Value, path: &str, expected: &str) -> bool {
    let parts: Vec<&str> = path.split('.').filter(|part| !part.is_empty()).collect();
    example_covers_path_parts(example, &parts, expected)
}

fn example_covers_path_parts(node: &Value, parts: &[&str], expected: &str) -> bool {
    if parts.is_empty() {
        return node.as_str() == Some(expected);
    }
    match node {
        Value::Array(items) => items
            .iter()
            .any(|item| example_covers_path_parts(item, parts, expected)),
        Value::Object(map) => {
            let Some(child) = map.get(parts[0]) else {
                return false;
            };
            example_covers_path_parts(child, &parts[1..], expected)
        }
        _ => false,
    }
}

/// Spec 102 FR-007: each `use_cases[i]` must have a matching `runtime-requests/ucNN-*.json`
/// fixture (1-based index, zero-padded to two digits).
#[cfg(test)]
fn use_case_smoke_coverage_gaps(
    use_case_count: usize,
    runtime_request_filenames: &[String],
) -> Vec<String> {
    let mut gaps = Vec::new();
    for index in 1..=use_case_count {
        let prefix = format!("uc{index:02}-");
        let found = runtime_request_filenames.iter().any(|name| {
            name.starts_with(&prefix)
                && Path::new(name)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
                && !name.contains('/')
        });
        if !found {
            gaps.push(format!(
                "use_cases[{}] lacks runtime-requests/{prefix}*.json (spec 102 FR-007)",
                index.saturating_sub(1)
            ));
        }
    }
    gaps
}

#[cfg(test)]
fn use_case_smoke_coverage_gaps_for_package(package_dir: &Path) -> Result<Vec<String>, String> {
    let contract_path = package_dir.join("contract.json");
    let contract_text = fs::read_to_string(&contract_path)
        .map_err(|error| format!("failed to read {}: {error}", contract_path.display()))?;
    let contract: Value = serde_json::from_str(&contract_text)
        .map_err(|error| format!("failed to parse {}: {error}", contract_path.display()))?;
    let use_case_count = contract
        .get("use_cases")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if use_case_count == 0 {
        return Ok(vec![
            "use_cases is missing or empty; cannot verify smoke coverage (spec 102 FR-004/FR-007)"
                .to_string(),
        ]);
    }
    let requests_dir = package_dir.join("runtime-requests");
    if !requests_dir.is_dir() {
        return Ok(vec![format!(
            "runtime-requests/ directory missing under {} (spec 102 FR-007)",
            package_dir.display()
        )]);
    }
    let mut filenames = Vec::new();
    for entry in fs::read_dir(&requests_dir)
        .map_err(|error| format!("failed to read {}: {error}", requests_dir.display()))?
    {
        let entry = entry.map_err(|error| {
            format!(
                "failed to read entry under {}: {error}",
                requests_dir.display()
            )
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if Path::new(&name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            filenames.push(name);
        }
    }
    filenames.sort();
    Ok(use_case_smoke_coverage_gaps(use_case_count, &filenames))
}

/// Resolve each `use_cases[].persona_ref` against `personas/<id>/<version>/persona.json`
/// in the target registry checkout so authors learn about gaps before a registry PR.
fn enforce_persona_refs_resolve(
    contract_text: &str,
    registry_repo_path: &Path,
) -> Result<(), (&'static str, String)> {
    let value: Value = serde_json::from_str(contract_text).map_err(|error| {
        (
            "capability_publish_contract_parse_failed",
            format!("failed to parse capability contract JSON: {error}"),
        )
    })?;
    match unresolved_persona_refs(&value, registry_repo_path) {
        Ok(missing) if missing.is_empty() => Ok(()),
        Ok(missing) => Err((
            "capability_publish_persona_ref_unresolved",
            format!(
                "use_cases[].persona_ref values are missing from the registry personas tree: {}. Expected personas/<id>/<version>/persona.json under {}",
                missing.join(", "),
                registry_repo_path.display()
            ),
        )),
        Err(message) => Err(("capability_publish_persona_ref_unresolved", message)),
    }
}

fn unresolved_persona_refs(
    contract: &Value,
    registry_repo_path: &Path,
) -> Result<Vec<String>, String> {
    let use_cases = contract
        .get("use_cases")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut required = std::collections::BTreeSet::new();
    for use_case in &use_cases {
        let Some(persona_ref) = use_case.get("persona_ref") else {
            continue;
        };
        let Some(persona_id) = persona_ref.as_str() else {
            return Err(
                "use_cases[].persona_ref must be a string id (for example platform-security-engineer)"
                    .to_string(),
            );
        };
        if persona_id.trim().is_empty()
            || persona_id == "."
            || persona_id == ".."
            || persona_id.contains('/')
            || persona_id.contains('\\')
        {
            return Err(format!(
                "use_cases[].persona_ref is not a safe persona id: {persona_id}"
            ));
        }
        required.insert(persona_id.to_string());
    }

    Ok(required
        .into_iter()
        .filter(|persona_id| !persona_exists_in_registry(registry_repo_path, persona_id))
        .collect())
}

fn persona_exists_in_registry(registry_repo_path: &Path, persona_id: &str) -> bool {
    let persona_root = registry_repo_path.join("personas").join(persona_id);
    let Ok(entries) = fs::read_dir(&persona_root) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && path.join("persona.json").is_file() {
            return true;
        }
    }
    false
}

fn validate_registry_path_segment(
    value: &str,
    label: &'static str,
) -> Result<(), (&'static str, String)> {
    if value.trim().is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
    {
        return Err((
            "capability_publish_invalid_registry_path",
            format!("{label} is not safe for a registry path: {value}"),
        ));
    }
    Ok(())
}

fn publish_file_sha256_digest(path: &Path) -> Result<String, (&'static str, String)> {
    let bytes = fs::read(path).map_err(|error| {
        (
            "capability_publish_artifact_read_failed",
            format!("failed to read capability artifact for digest computation: {error}"),
        )
    })?;
    Ok(format!("sha256:{}", sha256_hex(&bytes)))
}

fn publish_artifact_asset_name(path: &Path) -> Result<String, (&'static str, String)> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            (
                "capability_publish_artifact_name_invalid",
                format!(
                    "artifact path has no valid UTF-8 filename: {}",
                    path.display()
                ),
            )
        })?;
    if name.is_empty()
        || !name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
    {
        return Err((
            "capability_publish_artifact_name_invalid",
            format!("artifact filename is not safe for a GitHub release URL: {name}"),
        ));
    }
    Ok(name.to_string())
}

fn ensure_clean_registry_checkout(
    registry_repo_path: &Path,
    runner: &dyn PublishProcessRunner,
) -> Result<(), String> {
    let output = runner.run(
        registry_repo_path,
        "git",
        &["status".to_string(), "--porcelain".to_string()],
    )?;
    if !output.stdout.trim().is_empty() {
        return Err("registry checkout has uncommitted changes; commit, stash, or use a clean checkout before publishing".to_string());
    }
    Ok(())
}

fn prepare_and_open_registry_pr(
    request: &CapabilityPublishRequest,
    plan: &CapabilityPublishPlan,
    runner: &dyn PublishProcessRunner,
    registry_repo: &str,
) -> Result<String, (&'static str, String, Option<String>)> {
    // `gh release create` runs with cwd = registry checkout, so relative artifact
    // paths from the Traverse workspace fail with "no matches found". Always pass
    // an absolute path when one can be resolved.
    let artifact_for_release = request
        .artifact_path
        .canonicalize()
        .unwrap_or_else(|_| request.artifact_path.clone());
    run_publish_command(
        runner,
        &request.registry_repo_path,
        "gh",
        &[
            "release".to_string(),
            "create".to_string(),
            plan.artifact_release_tag.clone(),
            artifact_for_release.display().to_string(),
            "--repo".to_string(),
            registry_repo.to_string(),
            "--title".to_string(),
            format!("{} {} artifact", plan.capability_id, plan.version),
            "--notes".to_string(),
            format!(
                "Artifact for governed capability publication `{}` version `{}`. Digest: `{}`.",
                plan.capability_id, plan.version, plan.artifact_digest
            ),
        ],
        "capability_publish_release_create_failed",
        plan,
    )?;
    run_publish_command(
        runner,
        &request.registry_repo_path,
        "git",
        &[
            "checkout".to_string(),
            "-B".to_string(),
            plan.branch.clone(),
        ],
        "capability_publish_branch_failed",
        plan,
    )?;
    write_registry_contract(plan)?;
    run_publish_command(
        runner,
        &request.registry_repo_path,
        "git",
        &[
            "add".to_string(),
            plan.registry_relative_path.display().to_string(),
        ],
        "capability_publish_git_add_failed",
        plan,
    )?;
    run_publish_command(
        runner,
        &request.registry_repo_path,
        "git",
        &[
            "commit".to_string(),
            "-m".to_string(),
            format!("Publish {} {}", plan.capability_id, plan.version),
        ],
        "capability_publish_git_commit_failed",
        plan,
    )?;
    run_publish_command(
        runner,
        &request.registry_repo_path,
        "git",
        &[
            "push".to_string(),
            "-u".to_string(),
            "origin".to_string(),
            plan.branch.clone(),
        ],
        "capability_publish_git_push_failed",
        plan,
    )?;
    let body = capability_publish_pr_body(plan);
    let output = run_publish_command(
        runner,
        &request.registry_repo_path,
        "gh",
        &[
            "pr".to_string(),
            "create".to_string(),
            "--repo".to_string(),
            registry_repo.to_string(),
            "--base".to_string(),
            "main".to_string(),
            "--head".to_string(),
            plan.branch.clone(),
            "--title".to_string(),
            plan.title.clone(),
            "--body".to_string(),
            body,
        ],
        "capability_publish_pr_create_failed",
        plan,
    )?;
    Ok(output.stdout)
}

fn write_registry_contract(
    plan: &CapabilityPublishPlan,
) -> Result<(), (&'static str, String, Option<String>)> {
    if let Some(parent) = plan.registry_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            (
                "capability_publish_write_failed",
                format!("failed to create registry target directory: {error}"),
                Some(partial_state(plan)),
            )
        })?;
    }
    fs::write(&plan.registry_path, format!("{}\n", plan.contract_json)).map_err(|error| {
        (
            "capability_publish_write_failed",
            format!("failed to write registry contract: {error}"),
            Some(partial_state(plan)),
        )
    })
}

fn run_publish_command(
    runner: &dyn PublishProcessRunner,
    cwd: &Path,
    program: &str,
    args: &[String],
    code: &'static str,
    plan: &CapabilityPublishPlan,
) -> Result<PublishCommandOutput, (&'static str, String, Option<String>)> {
    runner
        .run(cwd, program, args)
        .map_err(|error| (code, error, Some(partial_state(plan))))
}

fn capability_publish_pr_body(plan: &CapabilityPublishPlan) -> String {
    // Governing specs must be from the *registry* approved-spec set, not Traverse's.
    // Declaring Traverse-only IDs (056/054/102) fails registry `spec-alignment`.
    format!(
        "## Summary\n\n- publish `{}` version `{}` to the public capability registry\n- add `{}`\n\n## Governing Spec\n\n- `001-registry-foundation`\n- `002-capability-validation`\n- `005-yank-deprecation`\n- `006-public-scope-and-identity`\n- `007-artifact-hosting`\n\n## Project Item\n\n- Capability publish via traverse-cli\n\n## Validation\n\n- local capability contract validation passed\n- contract surface coverage (schema ⊆ use_cases) passed\n- use_cases persona_ref resolution against registry personas passed\n- artifact digest computed: `{}`\n- release artifact: `{}`\n",
        plan.capability_id,
        plan.version,
        plan.registry_relative_path.display(),
        plan.artifact_digest,
        plan.artifact_url
    )
}

fn partial_state(plan: &CapabilityPublishPlan) -> String {
    format!(
        "release `{}` may contain `{}` and branch `{}` may contain `{}`; retry after fixing the reported error or clean up the branch manually",
        plan.artifact_release_tag,
        plan.artifact_asset_name,
        plan.branch,
        plan.registry_relative_path.display()
    )
}

fn capability_publish_success_json(
    status: &str,
    plan: &CapabilityPublishPlan,
    pull_request_url: Option<&str>,
    registry_repo: &str,
) -> Result<String, CliError> {
    let mut value = serde_json::json!({
        "status": status,
        "registry_repo": registry_repo,
        "branch": plan.branch,
        "registry_path": plan.registry_relative_path.display().to_string(),
        "namespace": plan.namespace,
        "capability_id": plan.capability_id,
        "version": plan.version,
        "artifact_digest": plan.artifact_digest,
        "artifact_release_tag": plan.artifact_release_tag,
        "artifact_url": plan.artifact_url,
        "validation_status": "passed"
    });
    if let Some(url) = pull_request_url {
        value["pull_request_url"] = Value::String(url.to_string());
    }
    serde_json::to_string_pretty(&value).map_err(|error| {
        CliError::IoError(format!(
            "failed to serialize capability publish output: {error}"
        ))
    })
}

fn capability_publish_failure_json(
    code: &str,
    message: &str,
    plan: Option<&CapabilityPublishPlan>,
    partial_state: Option<&str>,
    pull_request_url: Option<&str>,
    registry_repo: &str,
) -> Result<String, CliError> {
    let mut value = serde_json::json!({
        "status": "failed",
        "registry_repo": registry_repo,
        "errors": [{
            "code": code,
            "message": message,
            "severity": "error"
        }]
    });
    if let Some(plan) = plan {
        value["branch"] = Value::String(plan.branch.clone());
        value["registry_path"] = Value::String(plan.registry_relative_path.display().to_string());
        value["artifact_digest"] = Value::String(plan.artifact_digest.clone());
    }
    if let Some(partial_state) = partial_state {
        value["partial_state"] = Value::String(partial_state.to_string());
    }
    if let Some(url) = pull_request_url {
        value["pull_request_url"] = Value::String(url.to_string());
    }
    serde_json::to_string_pretty(&value).map_err(|error| {
        CliError::IoError(format!(
            "failed to serialize capability publish failure: {error}"
        ))
    })
}

fn sanitize_branch_component(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            output.push(ch);
        } else {
            output.push('-');
        }
    }
    output.trim_matches('-').to_string()
}

fn latest_index_release_asset(
    releases_json: &Value,
) -> Result<(String, String), RegistrySyncError> {
    let releases = releases_json.as_array().ok_or_else(|| {
        RegistrySyncError::new(
            "registry_release_parse_failed",
            "GitHub Releases response must be an array".to_string(),
        )
    })?;

    let mut selected: Option<(u64, String, String)> = None;
    for release in releases {
        let Some(tag) = release.get("tag_name").and_then(Value::as_str) else {
            continue;
        };
        let Some(index_version) = tag.strip_prefix("index-v").and_then(parse_u64) else {
            continue;
        };
        let Some(asset_url) = release
            .get("assets")
            .and_then(Value::as_array)
            .and_then(|assets| index_asset_download_url(assets))
        else {
            continue;
        };
        match &selected {
            Some((selected_version, _, _)) if *selected_version >= index_version => {}
            _ => selected = Some((index_version, tag.to_string(), asset_url)),
        }
    }

    selected
        .map(|(_, tag, asset_url)| (tag, asset_url))
        .ok_or_else(|| {
            RegistrySyncError::new(
                "registry_index_release_missing",
                "no index-v* release with an index.json asset was found".to_string(),
            )
        })
}

fn index_asset_download_url(assets: &[Value]) -> Option<String> {
    assets.iter().find_map(|asset| {
        let name = asset.get("name").and_then(Value::as_str)?;
        if name != "index.json" {
            return None;
        }
        asset
            .get("browser_download_url")
            .and_then(Value::as_str)
            .map(ToString::to_string)
    })
}

fn parse_u64(value: &str) -> Option<u64> {
    value.parse::<u64>().ok()
}

fn curl_text(url: &str, token: Option<&str>) -> Result<String, RegistrySyncError> {
    let mut args = vec![
        "-fsSL".to_string(),
        "-H".to_string(),
        "Accept: application/vnd.github+json".to_string(),
        "-H".to_string(),
        "User-Agent: traverse-cli-registry-sync".to_string(),
    ];
    if let Some(token) = token {
        args.push("-H".to_string());
        args.push(format!("Authorization: Bearer {token}"));
    }
    args.push(url.to_string());
    let output = std::process::Command::new("curl")
        .args(&args)
        .output()
        .map_err(|error| {
            RegistrySyncError::new(
                "registry_fetch_failed",
                format!("failed to execute curl for {url}: {error}"),
            )
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("curl exited with status {}", output.status)
        } else {
            stderr
        };
        return Err(RegistrySyncError::new(
            "registry_fetch_failed",
            format!("failed to fetch {url}: {detail}"),
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| {
        RegistrySyncError::new(
            "registry_fetch_failed",
            format!("response from {url} was not valid UTF-8: {error}"),
        )
    })
}

fn current_unix_timestamp_string() -> Result<String, CliError> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| {
            CliError::IoError(format!("system clock is before Unix epoch: {error}"))
        })?;
    Ok(format!("unix:{}", duration.as_secs()))
}

fn render_app_registration_state(
    manifest_path: &Path,
    workspace_id: &str,
    manifest: &traverse_registry::ApplicationBundleManifest,
) -> Result<Value, AppValidationError> {
    let manifest_digest = file_sha256_digest(manifest_path)?;
    let component_ids = app_registration_component_ids(manifest);
    let workflow_ids = app_registration_workflow_ids(manifest);
    let components = app_registration_components(manifest);
    let workflows = app_registration_workflows(manifest_path, manifest)?;
    let digest_verification = app_registration_digest_verification(manifest);
    let model_readiness = app_registration_model_readiness(manifest);
    let model_dependencies = manifest.model_dependencies.clone();
    let bundle_fingerprint = serde_json::json!({
        "app_id": manifest.app_id.clone(),
        "app_version": manifest.version.clone(),
        "manifest_digest": manifest_digest.clone(),
        "components": components.clone(),
        "workflows": workflows.clone(),
        "model_dependencies": model_dependencies.clone(),
        "model_readiness": model_readiness.clone(),
        "effective_config": {
            "values": manifest.effective_config.values.clone(),
            "redacted_secret_keys": manifest.effective_config.redacted_secret_keys.clone()
        }
    });
    let bundle_digest = value_sha256_digest(&bundle_fingerprint);

    Ok(serde_json::json!({
        "status": "registered",
        "workspace_id": workspace_id,
        "app_id": manifest.app_id.clone(),
        "app_version": manifest.version.clone(),
        "schema_version": manifest.schema_version.clone(),
        "manifest_path": manifest_path.display().to_string(),
        "manifest_digest": manifest_digest,
        "bundle_digest": bundle_digest,
        "component_ids": component_ids,
        "workflow_ids": workflow_ids,
        "components": components,
        "workflows": workflows,
        "digest_verification": digest_verification,
        "model_dependencies": model_dependencies,
        "model_readiness": model_readiness,
        "effective_config": {
            "values": manifest.effective_config.values.clone(),
            "redacted_secret_keys": manifest.effective_config.redacted_secret_keys.clone()
        },
        "runtime_references": {
            "inspection": format!("/v1/apps/{}/{}", manifest.app_id, manifest.version),
            "workflows": manifest.workflows.iter().map(|workflow| {
                format!("/v1/workflows/{}/{}", workflow.workflow_id, workflow.workflow_version)
            }).collect::<Vec<_>>()
        },
        "public_surfaces": manifest.public_surfaces.clone(),
        "state_machine": manifest.state_machine.clone(),
        "state_scope": "workspace_persisted",
        "state_path": app_registration_relative_state_path(
            workspace_id,
            &manifest.app_id,
            &manifest.version
        ).display().to_string(),
        "registration_fingerprint": bundle_fingerprint,
        "governing_specs": [
            "044-application-bundle-manifest",
            "045-governed-model-dependency-resolution",
            "046-public-cli-app-registration"
        ]
    }))
}

fn app_registration_component_ids(
    manifest: &traverse_registry::ApplicationBundleManifest,
) -> Vec<String> {
    manifest
        .components
        .iter()
        .map(|component| component.manifest.component_id.clone())
        .collect()
}

fn app_registration_workflow_ids(
    manifest: &traverse_registry::ApplicationBundleManifest,
) -> Vec<String> {
    manifest
        .workflows
        .iter()
        .map(|workflow| workflow.workflow_id.clone())
        .collect()
}

fn app_registration_components(
    manifest: &traverse_registry::ApplicationBundleManifest,
) -> Vec<Value> {
    manifest
        .components
        .iter()
        .map(|component| {
            serde_json::json!({
                "component_id": component.manifest.component_id.clone(),
                "component_version": component.manifest.version.clone(),
                "execution_mode": component.manifest.execution_mode.as_str(),
                "capability_id": component.manifest.capability_id.clone(),
                "capability_version": component.manifest.capability_version.clone(),
                "wasm_digest": component.verified_wasm_digest.clone(),
                "manifest_path": component.manifest_path.display().to_string(),
                "contract_path": component.contract_path.display().to_string(),
                "artifact_ref": component.wasm_binary_path.as_ref().map(|path| path.display().to_string()),
                "platforms": component.manifest.platforms.clone(),
                "wrapper_path": component.manifest.wrapper_path.clone()
            })
        })
        .collect()
}

fn app_registration_workflows(
    manifest_path: &Path,
    manifest: &traverse_registry::ApplicationBundleManifest,
) -> Result<Vec<Value>, AppValidationError> {
    manifest
        .workflows
        .iter()
        .map(|workflow| {
            let workflow_path = manifest_path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(&workflow.path);
            let workflow_digest = file_sha256_digest(&workflow_path)?;
            Ok(serde_json::json!({
                "workflow_id": workflow.workflow_id.clone(),
                "workflow_version": workflow.workflow_version.clone(),
                "workflow_digest": workflow_digest,
                "path": workflow_path.display().to_string()
            }))
        })
        .collect()
}

fn app_registration_digest_verification(
    manifest: &traverse_registry::ApplicationBundleManifest,
) -> Vec<Value> {
    manifest
        .components
        .iter()
        .filter_map(|component| {
            component.wasm_binary_path.as_ref().map(|wasm_binary_path| {
                serde_json::json!({
                    "component_id": component.manifest.component_id.clone(),
                    "component_version": component.manifest.version.clone(),
                    "path": wasm_binary_path.display().to_string(),
                    "wasm_digest": component.verified_wasm_digest.clone(),
                    "status": "verified"
                })
            })
        })
        .collect()
}

fn app_registration_model_readiness(
    manifest: &traverse_registry::ApplicationBundleManifest,
) -> Vec<Value> {
    manifest
        .model_dependencies
        .iter()
        .map(|dependency| {
            serde_json::json!({
                "interface_id": dependency.interface_id.clone(),
                "version_range": dependency.version_range.clone(),
                "selection_strategy": dependency.selection_policy.strategy.clone(),
                "candidate_count": dependency.candidates.len(),
                "candidate_ids": dependency.candidates.iter().map(|candidate| candidate.candidate_id.clone()).collect::<Vec<_>>(),
                "status": "declared"
            })
        })
        .collect()
}

fn read_existing_registration_fingerprint(path: &Path) -> Result<Option<Value>, CliError> {
    if !path.exists() {
        return Ok(None);
    }
    let state = read_json_file(path)?;
    Ok(Some(
        state
            .get("registration_fingerprint")
            .cloned()
            .unwrap_or(Value::Null),
    ))
}

fn file_sha256_digest(path: &Path) -> Result<String, AppValidationError> {
    let bytes = fs::read(path).map_err(|error| AppValidationError {
        code: "workspace_state_digest_failed".to_string(),
        path: path.display().to_string(),
        message: format!("failed to read artifact for registration digest: {error}"),
    })?;
    Ok(format!("sha256:{}", sha256_hex(&bytes)))
}

fn value_sha256_digest(value: &Value) -> String {
    format!("sha256:{}", sha256_hex(value.to_string().as_bytes()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

fn app_registration_state_path(
    base_dir: &Path,
    workspace_id: &str,
    app_id: &str,
    version: &str,
) -> PathBuf {
    base_dir.join(app_registration_relative_state_path(
        workspace_id,
        app_id,
        version,
    ))
}

fn app_registration_relative_state_path(
    workspace_id: &str,
    app_id: &str,
    version: &str,
) -> PathBuf {
    PathBuf::from(".traverse")
        .join("workspaces")
        .join(workspace_id)
        .join("apps")
        .join(sanitize_state_segment(app_id))
        .join(sanitize_state_segment(version))
        .join("registration.json")
}

fn app_activation_state_path(
    base_dir: &Path,
    workspace_id: &str,
    app_id: &str,
    version: &str,
) -> PathBuf {
    base_dir
        .join(".traverse")
        .join("workspaces")
        .join(workspace_id)
        .join("apps")
        .join(sanitize_state_segment(app_id))
        .join(sanitize_state_segment(version))
        .join("activation.json")
}

fn sanitize_state_segment(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn validate_workspace_id_for_cli(workspace_id: &str) -> Option<AppValidationError> {
    let valid = !workspace_id.is_empty()
        && !workspace_id.contains("..")
        && workspace_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'));
    if valid {
        None
    } else {
        Some(AppValidationError {
            code: "invalid_workspace_id".to_string(),
            path: "$.workspace_id".to_string(),
            message:
                "workspace id must contain only ASCII letters, digits, dot, dash, or underscore"
                    .to_string(),
        })
    }
}

fn write_registration_state_atomically(
    state_path: &Path,
    state: &Value,
) -> Result<(), AppValidationError> {
    let Some(parent) = state_path.parent() else {
        return Err(AppValidationError {
            code: "workspace_state_write_failed".to_string(),
            path: state_path.display().to_string(),
            message: "registration state path has no parent directory".to_string(),
        });
    };
    if let Err(error) = fs::create_dir_all(parent) {
        return Err(AppValidationError {
            code: "workspace_state_write_failed".to_string(),
            path: parent.display().to_string(),
            message: format!("failed to create workspace state directory: {error}"),
        });
    }

    let serialized = serde_json::to_string_pretty(state).map_err(|error| AppValidationError {
        code: "workspace_state_write_failed".to_string(),
        path: state_path.display().to_string(),
        message: format!("failed to serialize workspace registration state: {error}"),
    })?;
    let tmp_path = state_path.with_file_name("registration.json.tmp");
    if let Err(error) = fs::write(&tmp_path, format!("{serialized}\n")) {
        return Err(AppValidationError {
            code: "workspace_state_write_failed".to_string(),
            path: tmp_path.display().to_string(),
            message: format!("failed to write temporary registration state: {error}"),
        });
    }
    if let Err(error) = fs::rename(&tmp_path, state_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(AppValidationError {
            code: "workspace_state_write_failed".to_string(),
            path: state_path.display().to_string(),
            message: format!("failed to commit registration state atomically: {error}"),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AppValidationError {
    code: String,
    path: String,
    message: String,
}

impl AppValidationError {
    fn from_manifest_error(error: traverse_registry::ApplicationManifestError) -> Self {
        Self {
            code: debug_enum_to_snake_case(&format!("{:?}", error.code)),
            path: error.path,
            message: error.message,
        }
    }
}

fn render_app_validation_success(
    manifest_path: &Path,
    manifest: &traverse_registry::ApplicationBundleManifest,
) -> Result<String, CliError> {
    let component_ids = manifest
        .components
        .iter()
        .map(|component| component.manifest.component_id.clone())
        .collect::<Vec<_>>();
    let workflow_ids = manifest
        .workflows
        .iter()
        .map(|workflow| workflow.workflow_id.clone())
        .collect::<Vec<_>>();
    let digest_results = manifest
        .components
        .iter()
        .filter_map(|component| {
            component.wasm_binary_path.as_ref().map(|wasm_binary_path| {
                serde_json::json!({
                    "component_id": component.manifest.component_id.clone(),
                    "component_version": component.manifest.version.clone(),
                    "path": wasm_binary_path.display().to_string(),
                    "wasm_digest": component.verified_wasm_digest.clone(),
                    "status": "verified"
                })
            })
        })
        .collect::<Vec<_>>();
    let model_dependencies = manifest
        .model_dependencies
        .iter()
        .map(|dependency| {
            serde_json::json!({
                "interface_id": dependency.interface_id.clone(),
                "version_range": dependency.version_range.clone(),
                "selection_strategy": dependency.selection_policy.strategy.clone(),
                "candidate_count": dependency.candidates.len(),
                "candidate_ids": dependency.candidates.iter().map(|candidate| candidate.candidate_id.clone()).collect::<Vec<_>>(),
                "status": "declared"
            })
        })
        .collect::<Vec<_>>();

    let output = serde_json::json!({
        "status": "validated",
        "app_id": manifest.app_id,
        "app_version": manifest.version,
        "schema_version": manifest.schema_version,
        "manifest_path": manifest_path.display().to_string(),
        "component_ids": component_ids,
        "workflow_ids": workflow_ids,
        "components": manifest.components.iter().map(|component| {
            serde_json::json!({
                "component_id": component.manifest.component_id.clone(),
                "component_version": component.manifest.version.clone(),
                "execution_mode": component.manifest.execution_mode.as_str(),
                "capability_id": component.manifest.capability_id.clone(),
                "capability_version": component.manifest.capability_version.clone(),
                "manifest_path": component.manifest_path.display().to_string(),
                "contract_path": component.contract_path.display().to_string(),
                "wasm_digest": component.verified_wasm_digest.clone(),
                "platforms": component.manifest.platforms.clone(),
                "wrapper_path": component.manifest.wrapper_path.clone()
            })
        }).collect::<Vec<_>>(),
        "workflows": manifest.workflows.iter().map(|workflow| {
            serde_json::json!({
                "workflow_id": workflow.workflow_id.clone(),
                "workflow_version": workflow.workflow_version.clone(),
                "path": workflow.path.clone()
            })
        }).collect::<Vec<_>>(),
        "digest_verification": digest_results,
        "model_readiness": model_dependencies,
        "effective_config": {
            "values": manifest.effective_config.values.clone(),
            "redacted_secret_keys": manifest.effective_config.redacted_secret_keys.clone()
        },
        "public_surfaces": manifest.public_surfaces.clone(),
        "state_machine": manifest.state_machine.clone(),
        "runtime_references": {
            "inspection": format!("/v1/apps/{}/{}", manifest.app_id, manifest.version),
            "workflows": manifest.workflows.iter().map(|workflow| {
                format!("/v1/workflows/{}/{}", workflow.workflow_id, workflow.workflow_version)
            }).collect::<Vec<_>>()
        },
        "governing_specs": [
            "044-application-bundle-manifest",
            "045-governed-model-dependency-resolution",
            "046-public-cli-app-registration"
        ]
    });
    serde_json::to_string_pretty(&output)
        .map_err(|e| CliError::IoError(format!("failed to serialize app validation: {e}")))
}

fn render_app_validation_failure(
    manifest_path: &Path,
    errors: Vec<AppValidationError>,
) -> Result<String, CliError> {
    let output = serde_json::json!({
        "status": "failed",
        "manifest_path": manifest_path.display().to_string(),
        "errors": errors.into_iter().map(|error| {
            serde_json::json!({
                "code": error.code,
                "path": error.path,
                "severity": "error",
                "message": error.message
            })
        }).collect::<Vec<_>>()
    });
    serde_json::to_string_pretty(&output)
        .map_err(|e| CliError::IoError(format!("failed to serialize app validation failure: {e}")))
}

fn render_app_registration_failure(
    manifest_path: &Path,
    workspace_id: &str,
    errors: Vec<AppValidationError>,
    state_path: Option<&Path>,
) -> Result<String, CliError> {
    let output = serde_json::json!({
        "status": "failed",
        "manifest_path": manifest_path.display().to_string(),
        "workspace_id": workspace_id,
        "state_path": state_path.map(|path| path.display().to_string()),
        "errors": errors.into_iter().map(|error| {
            serde_json::json!({
                "code": error.code,
                "path": error.path,
                "severity": "error",
                "message": error.message
            })
        }).collect::<Vec<_>>()
    });
    serde_json::to_string_pretty(&output).map_err(|e| {
        CliError::IoError(format!("failed to serialize app registration failure: {e}"))
    })
}

fn validate_app_manifest_metadata_for_cli(
    manifest_path: &Path,
) -> Result<Option<AppValidationError>, CliError> {
    let manifest = read_json_file(manifest_path)?;
    if let Some(error) = find_private_manifest_field(&manifest, "$") {
        return Ok(Some(error));
    }

    let manifest_dir = manifest_path.parent().unwrap_or_else(|| Path::new(""));
    let Some(components) = manifest.get("components").and_then(Value::as_array) else {
        return Ok(None);
    };

    for component in components {
        if let Some(digest) = component.get("digest").and_then(Value::as_str)
            && let Some(error) = validate_non_placeholder_sha256(
                "$.components[].digest",
                digest,
                "application component reference",
            )
        {
            return Ok(Some(error));
        }

        let Some(component_manifest_path) = component.get("manifest_path").and_then(Value::as_str)
        else {
            continue;
        };
        let component_path = manifest_dir.join(component_manifest_path);
        if !component_path.is_file() {
            continue;
        }
        let component_manifest = read_json_file(&component_path)?;
        if let Some(error) = find_private_manifest_field(&component_manifest, "$.components[]") {
            return Ok(Some(error));
        }
        if let Some(digest) = component_manifest
            .get("wasm_digest")
            .and_then(Value::as_str)
            && let Some(error) = validate_non_placeholder_sha256(
                &format!("{}:$.wasm_digest", component_path.display()),
                digest,
                "component manifest",
            )
        {
            return Ok(Some(error));
        }

        if let Some(error) =
            validate_component_risk_policy_for_cli(&component_path, &component_manifest)
        {
            return Ok(Some(error));
        }
    }

    Ok(None)
}

/// Spec 109 FR-005: a component manifest may declare `risk_policy` to narrow
/// (never widen) the egress connectors its capability's immutable, contract-
/// declared `risk` metadata allows. Silently skips the check when either the
/// contract or an override is absent/unreadable — a missing/invalid contract
/// reference is reported by other manifest validation, not this one.
fn validate_component_risk_policy_for_cli(
    component_path: &Path,
    component_manifest: &Value,
) -> Option<AppValidationError> {
    let egress_allowed_connectors = component_manifest
        .get("risk_policy")
        .and_then(|policy| policy.get("egress_allowed_connectors"))
        .and_then(Value::as_array)?;
    let egress_allowed_connectors: Vec<String> = egress_allowed_connectors
        .iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect();

    let contract_path = component_manifest
        .get("contract_path")
        .and_then(Value::as_str)?;
    let component_dir = component_path.parent().unwrap_or_else(|| Path::new(""));
    let resolved_contract_path = component_dir.join(contract_path);
    if !resolved_contract_path.is_file() {
        return None;
    }
    let contract_json = fs::read_to_string(&resolved_contract_path).ok()?;
    let contract = traverse_contracts::parse_contract(&contract_json).ok()?;

    let policy = traverse_contracts::ManifestRiskPolicy {
        egress_allowed_connectors: Some(egress_allowed_connectors),
    };
    let failure =
        traverse_contracts::validate_manifest_risk_policy(&contract.risk, &policy).err()?;
    let error = failure.errors.into_iter().next()?;
    Some(AppValidationError {
        code: "risk_policy_weakened".to_string(),
        path: format!("{}:{}", component_path.display(), error.path),
        message: error.message,
    })
}

fn find_private_manifest_field(value: &Value, path: &str) -> Option<AppValidationError> {
    let object = value.as_object()?;
    for key in object.keys() {
        let private = key.starts_with('_')
            || key.starts_with("internal")
            || key.starts_with("x-internal")
            || key.starts_with("private")
            || key.starts_with("x-private");
        if private {
            return Some(AppValidationError {
                code: "unsupported_private_field".to_string(),
                path: format!("{path}.{key}"),
                message: format!("unsupported private/internal manifest field {key}"),
            });
        }
    }
    None
}

fn validate_non_placeholder_sha256(
    path: &str,
    value: &str,
    artifact_kind: &str,
) -> Option<AppValidationError> {
    let digest = value.strip_prefix("sha256:").unwrap_or(value);
    let all_zero = digest.len() == 64 && digest.bytes().all(|byte| byte == b'0');
    let empty_sha256 = digest == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    if all_zero || empty_sha256 {
        return Some(AppValidationError {
            code: "placeholder_wasm_digest".to_string(),
            path: path.to_string(),
            message: format!("{artifact_kind} declares a placeholder or all-zero WASM digest"),
        });
    }
    None
}

fn component_namespace(component_id: &str) -> String {
    component_id
        .rsplit_once('.')
        .map_or(component_id.to_string(), |(namespace, _)| {
            namespace.to_string()
        })
}

fn scaffold_leaf_name(id: &str) -> String {
    id.rsplit_once('.')
        .map_or(id.to_string(), |(_, name)| name.to_string())
}

fn validate_scaffold_id(value: &str, label: &str) -> Result<(), CliError> {
    let valid = !value.is_empty()
        && !value.contains('/')
        && !value.contains('\\')
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'));
    if valid {
        Ok(())
    } else {
        Err(CliError::UsageError(format!(
            "{label} must contain only ASCII letters, digits, dot, dash, or underscore"
        )))
    }
}

fn write_pretty_json(path: &Path, value: &Value) -> Result<(), CliError> {
    let contents = serde_json::to_string_pretty(value)
        .map_err(|e| CliError::IoError(format!("failed to serialize JSON: {e}")))?;
    write_new_file(path, &format!("{contents}\n"))
}

fn write_new_file(path: &Path, contents: &str) -> Result<(), CliError> {
    if path.exists() {
        return Err(CliError::IoError(format!(
            "refusing to overwrite existing file: {}",
            path.display()
        )));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            CliError::IoError(format!(
                "failed to create parent directory {}: {e}",
                parent.display()
            ))
        })?;
    }
    fs::write(path, contents)
        .map_err(|e| CliError::IoError(format!("failed to write {}: {e}", path.display())))
}

fn render_application_registration_failure(
    errors: Vec<traverse_registry::ApplicationRegistrationError>,
) -> String {
    let details = errors
        .into_iter()
        .map(|error| format!("{:?}: {} ({})", error.code, error.message, error.path))
        .collect::<Vec<_>>()
        .join("; ");
    format!("application registration failed: {details}")
}

fn discover_capabilities(manifest_path: &Path, json_output: bool) -> Result<String, CliError> {
    let registered = load_registered_bundle(manifest_path)?;
    let entries = registered
        .capability_registry
        .discover(LookupScope::PreferPrivate, &DiscoveryQuery::default());

    if json_output {
        let json_entries: Vec<serde_json::Value> = entries
            .iter()
            .map(|entry| {
                let resolved = registered.capability_registry.find_exact(
                    LookupScope::PreferPrivate,
                    &entry.id,
                    &entry.version,
                );
                let package_mode = resolved
                    .as_ref()
                    .map_or("unknown", |capability| {
                        capability_discovery_package_mode(&capability.artifact)
                    });
                let advisory_compositions = resolved.as_ref().map_or_else(Vec::new, |capability| {
                    capability_discovery_advisory_compositions(
                        capability.artifact.workflow_ref.as_ref(),
                    )
                });
                serde_json::json!({
                    "id": entry.id,
                    "version": entry.version,
                    "scope": format!("{:?}", entry.scope).to_lowercase(),
                    "lifecycle": format!("{:?}", entry.lifecycle).to_lowercase(),
                    "implementation_kind": format!("{:?}", entry.implementation_kind).to_lowercase(),
                    "package_mode": package_mode,
                    "advisory_compositions": advisory_compositions,
                    "activation_eligibility": "unknown",
                    "activation_eligibility_reason": "requires_host_activation_resolution",
                    "summary": entry.summary,
                    "tags": entry.tags,
                })
            })
            .collect();
        serde_json::to_string_pretty(&serde_json::Value::Array(json_entries))
            .map_err(|e| CliError::IoError(format!("failed to serialize discovery results: {e}")))
    } else {
        let lines: Vec<String> = entries
            .iter()
            .map(|entry| format!("{}@{}", entry.id, entry.version))
            .collect();
        Ok(lines.join("\n"))
    }
}

fn capability_discovery_package_mode(artifact: &CapabilityArtifactRecord) -> &'static str {
    if artifact.workflow_ref.is_some() {
        "workflow_composed"
    } else {
        "standalone"
    }
}

fn capability_discovery_advisory_compositions(
    workflow_ref: Option<&WorkflowReference>,
) -> Vec<String> {
    workflow_ref
        .map(|reference| {
            vec![format!(
                "{}@{}",
                reference.workflow_id, reference.workflow_version
            )]
        })
        .unwrap_or_default()
}

fn inspect_capability_package(manifest_path: &Path) -> Result<String, CliError> {
    let package = load_capability_package(manifest_path).map_err(CliError::IoError)?;
    Ok(package.render_summary())
}

fn execute_capability_package(
    manifest_path: &Path,
    request_path: &Path,
) -> Result<String, CliError> {
    let package = load_capability_package(manifest_path).map_err(CliError::IoError)?;
    let request = load_runtime_request(request_path)?;
    let mut registry = CapabilityRegistry::new();
    registry
        .register(package.capability_registration())
        .map_err(|f| CliError::RegistrationConflict(render_registry_failure(f)))?;
    let sink: std::sync::Arc<dyn traverse_contracts::UsageTelemetrySink> =
        std::sync::Arc::from(telemetry::wire_usage_telemetry_sink());
    let runtime = Runtime::new(
        registry,
        ArtifactRouter::new().map_err(|error| CliError::ExecutionFailed(error.message))?,
    )
    .with_security_config(traverse_runtime::security::RuntimeSecurityConfig::development())
    .with_usage_telemetry_sink(sink.clone());
    let outcome = telemetry::execute_with_telemetry(&runtime, request, sink.as_ref());

    if outcome.result.status == RuntimeResultStatus::Error {
        return Err(CliError::ExecutionFailed(render_runtime_execution_failure(
            &outcome,
        )));
    }

    Ok(render_capability_package_execution_summary(
        &package.manifest.package_id,
        &package.manifest.capability_ref.id,
        &package.manifest.capability_ref.version,
        &outcome,
    ))
}

fn verify_wasm_abi_imports(wasm_paths: &[PathBuf]) -> Result<String, CliError> {
    let mut lines = Vec::new();
    for wasm_path in wasm_paths {
        let wasm_bytes = fs::read(wasm_path).map_err(|error| {
            CliError::IoError(format!(
                "failed to read WASM artifact {}: {error}",
                wasm_path.display()
            ))
        })?;
        let validation = verify_wasm_host_abi_bytes(&wasm_bytes, SUPPORTED_HOST_ABI_VERSION)
            .map_err(|error| {
                CliError::ValidationFailed(format!("{}: {error}", wasm_path.display()))
            })?;
        lines.push(format!(
            "{}: ABI {} import whitelist passed ({} imports)",
            wasm_path.display(),
            validation.abi_version,
            validation.imports.len()
        ));
    }

    Ok(lines.join("\n"))
}

fn verify_supply_chain_artifact(artifact_path: &Path) -> Result<String, CliError> {
    let report = supply_chain::verify_artifact(artifact_path);
    let json = serde_json::to_string_pretty(&report)
        .map_err(|e| CliError::IoError(format!("failed to serialize artifact report: {e}")))?;
    if report.passed() {
        Ok(json)
    } else {
        Err(CliError::ValidationFailed(json))
    }
}

fn sign_supply_chain_artifact(artifact_path: &Path) -> Result<String, CliError> {
    let report =
        supply_chain::sign_artifact(artifact_path).map_err(|e| CliError::IoError(e.to_string()))?;
    serde_json::to_string_pretty(&report)
        .map_err(|e| CliError::IoError(format!("failed to serialize signing report: {e}")))
}

fn execute_expedition(
    request_path: &Path,
    trace_output_path: Option<&Path>,
    json_output: bool,
    validate_only: bool,
) -> Result<String, CliError> {
    if validate_only {
        return validate_expedition_request(request_path);
    }

    let outcome = execute_expedition_outcome(request_path)?;

    if outcome.result.status == RuntimeResultStatus::Error {
        return Err(CliError::ExecutionFailed(render_runtime_execution_failure(
            &outcome,
        )));
    }

    if let Some(path) = trace_output_path {
        write_trace_artifact(path, &outcome.trace)?;
    }

    if json_output {
        serde_json::to_string_pretty(&outcome.trace)
            .map_err(|e| CliError::IoError(format!("failed to serialize runtime trace: {e}")))
    } else {
        Ok(render_runtime_execution_summary(
            &outcome,
            trace_output_path,
        ))
    }
}

fn validate_expedition_request(request_path: &Path) -> Result<String, CliError> {
    let request = load_runtime_request(request_path)?;
    let registered = load_registered_bundle(&canonical_expedition_bundle_path())?;

    let capability_id = request
        .intent
        .capability_id
        .as_deref()
        .unwrap_or("expedition.planning.plan-expedition");
    let capability_version = request
        .intent
        .capability_version
        .as_deref()
        .unwrap_or("1.0.0");

    if registered
        .capability_registry
        .find_exact(
            LookupScope::PreferPrivate,
            capability_id,
            capability_version,
        )
        .is_none()
    {
        return Err(CliError::ValidationFailed(format!(
            "capability {capability_id}@{capability_version} not found in registry"
        )));
    }

    Ok(format!(
        "validation passed: {capability_id}@{capability_version} is registered"
    ))
}

fn inspect_capability(contract_path: &Path) -> Result<String, CliError> {
    let contents = read_text_file(contract_path, "capability contract")?;
    let parsed = parse_contract(&contents).map_err(|failure| {
        CliError::ValidationFailed(render_validation_failure(
            "capability contract",
            contract_path,
            failure,
        ))
    })?;
    let validated = validate_contract(
        parsed,
        &ValidationContext {
            governing_spec: "002-capability-contracts",
            validator_version: env!("CARGO_PKG_VERSION"),
            existing_published: None,
        },
    )
    .map_err(|failure| {
        CliError::ValidationFailed(render_validation_failure(
            "capability contract",
            contract_path,
            failure,
        ))
    })?;

    Ok(render_capability_summary(
        contract_path,
        &validated.normalized,
    ))
}

fn inspect_event(contract_path: &Path) -> Result<String, CliError> {
    let contents = read_text_file(contract_path, "event contract")?;
    let parsed = parse_event_contract(&contents).map_err(|failure| {
        CliError::ValidationFailed(render_validation_failure(
            "event contract",
            contract_path,
            failure,
        ))
    })?;
    let validated = validate_event_contract(
        parsed,
        &EventValidationContext {
            governing_spec: "003-event-contracts",
            validator_version: env!("CARGO_PKG_VERSION"),
            existing_published: None,
        },
    )
    .map_err(|failure| {
        CliError::ValidationFailed(render_validation_failure(
            "event contract",
            contract_path,
            failure,
        ))
    })?;

    Ok(render_event_summary(contract_path, &validated.normalized))
}

fn validate_event_product(descriptor_path: &Path) -> Result<String, CliError> {
    let descriptor = traverse_runtime::events::validate_event_product_file(descriptor_path)
        .map_err(CliError::ValidationFailed)?;
    Ok(format!(
        "event_product_valid: true\nid: {}\nversion: {}\nexposure: {:?}\nsupport_route: {}\npublishers: {}\nsubscribers: {}",
        descriptor.contract.id,
        descriptor.contract.version,
        descriptor.exposure,
        descriptor.support_route,
        descriptor.contract.publishers.len(),
        descriptor.contract.subscribers.len()
    ))
}

fn workflow_register(workflow_path: &Path, workspace_id: &str) -> Result<String, CliError> {
    let workflow_json = read_text_file(workflow_path, "workflow definition")?;
    let workflow_value: serde_json::Value =
        serde_json::from_str(&workflow_json).map_err(|error| {
            CliError::ValidationFailed(format!(
                "failed to parse workflow JSON {}: {error}",
                workflow_path.display()
            ))
        })?;

    let registry_scope = if workspace_id == "system" {
        "public"
    } else {
        "private"
    };

    let body = serde_json::json!({
        "workspace_id": workspace_id,
        "scope": "workspace_persisted",
        "registry_scope": registry_scope,
        "workflow": workflow_value,
    })
    .to_string()
    .into_bytes();

    let (status, response) = build_in_process_api()?
        .register_workflow(body, true)
        .map_err(CliError::IoError)?;
    if status >= 400 {
        return Err(CliError::ValidationFailed(format!(
            "workflow registration failed: {response}"
        )));
    }

    Ok(format!(
        "workflow_id: {}\nversion: {}\ndigest: {}",
        response["workflow"]["id"].as_str().unwrap_or_default(),
        response["workflow"]["version"].as_str().unwrap_or_default(),
        response["workflow"]["digest"].as_str().unwrap_or_default(),
    ))
}

fn workflow_list(workspace_id: &str) -> Result<String, CliError> {
    let (status, response) = build_in_process_api()?
        .list_workflows(workspace_id, true)
        .map_err(CliError::IoError)?;
    if status >= 400 {
        return Err(CliError::ValidationFailed(format!(
            "workflow list failed: {response}"
        )));
    }

    let mut lines = Vec::new();
    lines.push(format!("workspace_id: {workspace_id}"));
    lines.push("workflows:".to_string());

    let Some(items) = response.as_array() else {
        return Err(CliError::ValidationFailed(
            "workflow list returned unexpected response shape".to_string(),
        ));
    };
    for item in items {
        let id = item.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        let version = item
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let digest = item
            .get("digest")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        lines.push(format!("  - {id}@{version} {digest}"));
    }

    Ok(lines.join("\n"))
}

fn workflow_inspect(
    workflow_id: &str,
    version: Option<&str>,
    workspace_id: &str,
) -> Result<String, CliError> {
    let (status, response) = build_in_process_api()?
        .get_workflow(workspace_id, workflow_id, version, true)
        .map_err(CliError::IoError)?;
    if status >= 400 {
        return Err(CliError::ValidationFailed(format!(
            "workflow inspect failed: {response}"
        )));
    }

    let workflow = response.get("workflow").cloned().unwrap_or_default();
    serde_json::to_string_pretty(&workflow)
        .map_err(|e| CliError::IoError(format!("failed to render workflow inspection output: {e}")))
}

fn build_in_process_api() -> Result<http_api::InProcessApi<ExpeditionExampleExecutor>, CliError> {
    let registered = load_registered_bundle(&canonical_expedition_bundle_path())?;
    http_api::InProcessApi::new(http_api::ApiServerConfig {
        bind_address: "127.0.0.1:0".to_string(),
        requested_auth_mode: None,
        allow_unauthenticated: true,
        allowed_origins: Vec::new(),
        render_mobile_qr: false,
        capability_registry: registered.capability_registry,
        workflow_registry: registered.workflow_registry,
        registry_root: std::env::current_dir()
            .map_err(|e| CliError::IoError(format!("failed to resolve current directory: {e}")))?
            .join(".traverse/registry"),
        executor: ExpeditionExampleExecutor,
        idempotency_retention_seconds: None,
        jwt_verification_key_hex: None,
        read_timeout: None,
        write_timeout: None,
        request_deadline: None,
        max_concurrent_connections: None,
        grpc_bind_address: None,
        grpc_tls_cert_path: None,
        grpc_tls_key_path: None,
    })
    .map_err(CliError::IoError)
}

fn inspect_trace(trace_path: &Path) -> Result<String, CliError> {
    let contents = read_text_file(trace_path, "runtime trace")?;
    let trace = serde_json::from_str::<RuntimeTrace>(&contents).map_err(|error| {
        CliError::ValidationFailed(format!(
            "failed to parse runtime trace {}: {error}",
            trace_path.display()
        ))
    })?;

    Ok(render_trace_summary(trace_path, &trace))
}

fn read_text_file(path: &Path, artifact_kind: &str) -> Result<String, CliError> {
    fs::read_to_string(path).map_err(|error| {
        CliError::IoError(format!(
            "failed to read {artifact_kind} {}: {error}",
            path.display()
        ))
    })
}

fn read_json_file(path: &Path) -> Result<Value, CliError> {
    let contents = read_text_file(path, "JSON file")?;
    serde_json::from_str(&contents).map_err(|error| {
        CliError::ValidationFailed(format!(
            "failed to parse JSON file {}: {error}",
            path.display()
        ))
    })
}

fn render_validation_failure(
    artifact_kind: &str,
    path: &Path,
    failure: traverse_contracts::ValidationFailure,
) -> String {
    let details = failure
        .errors
        .into_iter()
        .map(|error| format!("{} at {}", error.message, error.path))
        .collect::<Vec<_>>()
        .join("; ");

    format!(
        "failed to validate {artifact_kind} {}: {details}",
        path.display()
    )
}

fn render_bundle_summary(bundle: &RegistryBundle) -> String {
    let mut lines = vec![
        format!("bundle_id: {}", bundle.bundle_id),
        format!("version: {}", bundle.version),
        format!("scope: {:?}", bundle.scope).to_lowercase(),
        format!("capabilities: {}", bundle.capabilities.len()),
        format!("events: {}", bundle.events.len()),
        format!("workflows: {}", bundle.workflows.len()),
        "capability_ids:".to_string(),
    ];

    for capability in &bundle.capabilities {
        lines.push(format!(
            "  - {}@{}",
            capability.manifest.id, capability.manifest.version
        ));
    }

    lines.push("event_ids:".to_string());
    for event in &bundle.events {
        lines.push(format!(
            "  - {}@{}",
            event.manifest.id, event.manifest.version
        ));
    }

    lines.push("workflow_ids:".to_string());
    for workflow in &bundle.workflows {
        lines.push(format!(
            "  - {}@{}",
            workflow.manifest.id, workflow.manifest.version
        ));
    }

    lines.join("\n")
}

fn render_bundle_registration_summary(
    bundle: &RegistryBundle,
    capability_records: &[String],
    event_records: &[String],
    workflow_records: &[String],
    evidence: &[BundleRegistrationEvidence],
) -> String {
    let mut lines = vec![
        format!("bundle_id: {}", bundle.bundle_id),
        format!("version: {}", bundle.version),
        format!("scope: {:?}", bundle.scope).to_lowercase(),
        format!("registered_capabilities: {}", capability_records.len()),
        format!("registered_events: {}", event_records.len()),
        format!("registered_workflows: {}", workflow_records.len()),
        "capability_records:".to_string(),
    ];

    for record in capability_records {
        lines.push(format!("  - {record}"));
    }

    lines.push("event_records:".to_string());
    for record in event_records {
        lines.push(format!("  - {record}"));
    }

    lines.push("workflow_records:".to_string());
    for record in workflow_records {
        lines.push(format!("  - {record}"));
    }

    lines.push("evidence:".to_string());
    for item in evidence {
        lines.push(format!(
            "  - [{}] {}@{}: {}",
            item.code, item.capability_id, item.capability_version, item.message
        ));
    }

    lines.join("\n")
}

fn render_capability_summary(path: &Path, contract: &CapabilityContract) -> String {
    let input_properties = schema_property_count(&contract.inputs.schema);
    let output_properties = schema_property_count(&contract.outputs.schema);
    let mut lines = vec![
        format!("path: {}", path.display()),
        format!("id: {}", contract.id),
        format!("namespace: {}", contract.namespace),
        format!("name: {}", contract.name),
        format!("version: {}", contract.version),
        format!("lifecycle: {:?}", contract.lifecycle).to_lowercase(),
        format!("service_type: {:?}", contract.service_type).to_lowercase(),
        format!("summary: {}", contract.summary),
        format!("input_schema_properties: {input_properties}"),
        format!("output_schema_properties: {output_properties}"),
        format!("emits: {}", contract.emits.len()),
        format!("consumes: {}", contract.consumes.len()),
        format!("owner_team: {}", contract.owner.team),
        format!("provenance_author: {}", contract.provenance.author),
    ];
    if let Some(spec_ref) = &contract.provenance.spec_ref {
        lines.push(format!("provenance_spec_ref: {spec_ref}"));
    }
    lines.push(
        format!(
            "host_api_access: {:?}",
            contract.execution.constraints.host_api_access
        )
        .to_lowercase(),
    );
    lines.join("\n")
}

fn schema_property_count(schema: &Value) -> usize {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .map_or(0, serde_json::Map::len)
}

fn render_event_summary(path: &Path, contract: &EventContract) -> String {
    let mut lines = vec![
        format!("path: {}", path.display()),
        format!("id: {}", contract.id),
        format!("version: {}", contract.version),
        format!("lifecycle: {:?}", contract.lifecycle).to_lowercase(),
        format!("event_type: {:?}", contract.classification.event_type).to_lowercase(),
        format!("domain: {}", contract.classification.domain),
        format!(
            "bounded_context: {}",
            contract.classification.bounded_context
        ),
        format!("publishers: {}", contract.publishers.len()),
        format!("subscribers: {}", contract.subscribers.len()),
        format!("tags: {}", contract.tags.join(",")),
        "publisher_ids:".to_string(),
    ];

    for publisher in &contract.publishers {
        lines.push(format!(
            "  - {}@{}",
            publisher.capability_id, publisher.version
        ));
    }

    lines.push("subscriber_ids:".to_string());
    for subscriber in &contract.subscribers {
        lines.push(format!(
            "  - {}@{}",
            subscriber.capability_id, subscriber.version
        ));
    }

    lines.join("\n")
}

fn render_runtime_execution_summary(
    outcome: &RuntimeExecutionOutcome,
    trace_output_path: Option<&Path>,
) -> String {
    let output = outcome.result.output.as_ref().unwrap_or(&Value::Null);
    let mut lines = vec![
        format!("request_id: {}", outcome.result.request_id),
        format!("execution_id: {}", outcome.result.execution_id),
        "capability_id: expedition.planning.plan-expedition".to_string(),
        "capability_version: 1.0.0".to_string(),
        "status: completed".to_string(),
        format!("trace_ref: {}", outcome.result.trace_ref),
    ];

    if let Some(path) = trace_output_path {
        lines.push(format!("trace_path: {}", path.display()));
    }

    if let Some(plan_id) = output.get("plan_id").and_then(Value::as_str) {
        lines.push(format!("plan_id: {plan_id}"));
    }
    if let Some(objective_id) = output.get("objective_id").and_then(Value::as_str) {
        lines.push(format!("objective_id: {objective_id}"));
    }
    if let Some(route_style) = output
        .get("recommended_route_style")
        .and_then(Value::as_str)
    {
        lines.push(format!("recommended_route_style: {route_style}"));
    }
    if let Some(summary) = output.get("summary").and_then(Value::as_str) {
        lines.push(format!("summary: {summary}"));
    }

    lines.join("\n")
}

fn render_capability_package_execution_summary(
    package_id: &str,
    capability_id: &str,
    capability_version: &str,
    outcome: &RuntimeExecutionOutcome,
) -> String {
    format_capability_package_execution_summary(
        package_id,
        capability_id,
        capability_version,
        &outcome.result.request_id,
        &outcome.result.execution_id,
        &outcome.result.trace_ref,
        outcome.result.output.as_ref().unwrap_or(&Value::Null),
    )
}

fn format_capability_package_execution_summary(
    package_id: &str,
    capability_id: &str,
    capability_version: &str,
    request_id: &str,
    execution_id: &str,
    trace_ref: &str,
    output: &Value,
) -> String {
    [
        format!("request_id: {request_id}"),
        format!("execution_id: {execution_id}"),
        format!("package_id: {package_id}"),
        format!("capability_id: {capability_id}"),
        format!("capability_version: {capability_version}"),
        "status: completed".to_string(),
        format!("trace_ref: {trace_ref}"),
        "output:".to_string(),
        // Infallible pretty JSON via serde_json::Value's alternate Display.
        format!("{output:#}"),
    ]
    .join("\n")
}

fn render_trace_summary(trace_path: &Path, trace: &RuntimeTrace) -> String {
    let final_transition = trace.state_transitions.last();
    let mut lines = vec![
        format!("path: {}", trace_path.display()),
        format!("trace_id: {}", trace.trace_id),
        format!("execution_id: {}", trace.execution_id),
        format!("request_id: {}", trace.request_id),
        format!("governing_spec: {}", trace.governing_spec),
        format!("result_status: {:?}", trace.result.status).to_lowercase(),
        format!(
            "state_machine_validation: {:?}",
            trace.state_machine_validation.status
        )
        .to_lowercase(),
        format!("state_transition_count: {}", trace.state_transitions.len()),
        format!(
            "candidate_count: {}",
            trace.candidate_collection.candidates.len()
        ),
        format!(
            "rejected_candidate_count: {}",
            trace.candidate_collection.rejected_candidates.len()
        ),
        format!("execution_status: {:?}", trace.execution.status).to_lowercase(),
    ];

    if let Some(selected) = &trace.selection.selected_capability_id {
        lines.push(format!("selected_capability_id: {selected}"));
    }
    if let Some(version) = &trace.selection.selected_capability_version {
        lines.push(format!("selected_capability_version: {version}"));
    }
    if let Some(artifact_ref) = &trace.execution.artifact_ref {
        lines.push(format!("artifact_ref: {artifact_ref}"));
    }
    if let Some(transition) = final_transition {
        lines.push(format!(
            "terminal_transition: {} -> {} ({})",
            format!("{:?}", transition.from_state).to_lowercase(),
            format!("{:?}", transition.to_state).to_lowercase(),
            debug_enum_to_snake_case(&format!("{:?}", transition.reason_code))
        ));
    }
    if let Some(error) = &trace.result.error {
        lines.push(format!("error_code: {:?}", error.code).to_lowercase());
        lines.push(format!("error_message: {}", error.message));
    }

    lines.join("\n")
}

fn usage() -> String {
    "usage: traverse-cli app <new|validate|register> [options] | traverse-cli registry sync --workspace <id> --json | traverse-cli component new <component-id> | traverse-cli <bundle|capability-package|event|trace|workflow|expedition|federation> <inspect|register|execute|peers|sync|status> <artifact-path> [request-path] [--trace-out <trace-path>] | traverse-cli serve [--bind <address>] [--port <N>] [--allow-unauthenticated]".to_string()
}

fn write_trace_artifact(path: &Path, trace: &RuntimeTrace) -> Result<(), CliError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            CliError::IoError(format!(
                "failed to create trace artifact directory {}: {error}",
                parent.display()
            ))
        })?;
    }

    let serialized = serde_json::to_string_pretty(trace).map_err(|error| {
        CliError::IoError(format!(
            "failed to serialize runtime trace {}: {error}",
            path.display()
        ))
    })?;
    fs::write(path, format!("{serialized}\n")).map_err(|error| {
        CliError::IoError(format!(
            "failed to write runtime trace {}: {error}",
            path.display()
        ))
    })
}

fn debug_enum_to_snake_case(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 4);
    for (index, ch) in value.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index > 0 {
                output.push('_');
            }
            output.push(ch.to_ascii_lowercase());
        } else {
            output.push(ch);
        }
    }
    output
}

#[derive(Debug)]
struct RegisteredBundle {
    bundle: RegistryBundle,
    capability_registry: CapabilityRegistry,
    event_registry: EventRegistry,
    workflow_registry: WorkflowRegistry,
    capability_records: Vec<String>,
    event_records: Vec<String>,
    workflow_records: Vec<String>,
    evidence: Vec<BundleRegistrationEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct BundleRegistrationEvidence {
    code: &'static str,
    path: String,
    message: String,
    capability_id: String,
    capability_version: String,
    lookup_scope: &'static str,
}

#[derive(Debug, Default, Clone, Copy)]
struct ExpeditionExampleExecutor;

impl LocalExecutor for ExpeditionExampleExecutor {
    fn execute(
        &self,
        capability: &traverse_registry::ResolvedCapability,
        input: &Value,
    ) -> Result<LocalExecutionOutput, LocalExecutionFailure> {
        let value = match capability.contract.id.as_str() {
            "expedition.planning.capture-expedition-objective" => {
                execute_capture_expedition_objective(input)
            }
            "expedition.planning.interpret-expedition-intent" => {
                execute_interpret_expedition_intent(input)
            }
            "expedition.planning.assess-conditions-summary" => {
                execute_assess_conditions_summary(input)
            }
            "expedition.planning.validate-team-readiness" => execute_validate_team_readiness(input),
            "traverse-starter.process" => execute_traverse_starter_process(input),
            "traverse-starter.validate" => execute_traverse_starter_validate(input),
            "traverse-starter.summarize" => execute_traverse_starter_summarize(input),
            "meeting-notes.process" => execute_meeting_notes_process(input),
            "expedition.planning.assemble-expedition-plan" => {
                execute_assemble_expedition_plan(input)
            }
            other => Err(executor_failure(&format!(
                "unsupported expedition example capability: {other}"
            ))),
        }?;
        Ok(LocalExecutionOutput {
            value,
            emitted_events: Vec::new(),
        })
    }
}

fn build_capability_registration(
    bundle: &RegistryBundle,
    capability: &traverse_registry::CapabilityBundleArtifact,
) -> Result<CapabilityRegistration, CliError> {
    let raw_contract = read_text_file(&capability.path, "capability contract")?;
    let envelope =
        parse_capability_registration_envelope(&raw_contract, capability.path.as_path())?;
    let implementation_kind = derive_implementation_kind(envelope.get("composability"));
    let workflow_ref = derive_workflow_ref(envelope.get("composability"))?;
    let composability =
        derive_composability_metadata(implementation_kind, workflow_ref.as_ref(), capability)?;
    let artifact = build_capability_artifact(bundle, capability, implementation_kind, workflow_ref);

    Ok(CapabilityRegistration {
        scope: bundle.scope,
        contract: capability.contract.clone(),
        contract_path: capability.path.display().to_string(),
        artifact,
        registered_at: bundle_registered_at(bundle),
        tags: Vec::new(),
        composability,
        governing_spec: "005-capability-registry".to_string(),
        validator_version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

fn load_registered_bundle(manifest_path: &Path) -> Result<RegisteredBundle, CliError> {
    load_registered_bundle_with_public_records(manifest_path, &[])
}

fn load_governed_public_bundle(manifest_path: &Path) -> Result<RegisteredBundle, CliError> {
    load_registered_bundle_with_policy(manifest_path, &[], false)
}

fn load_registered_bundle_with_public_records(
    manifest_path: &Path,
    public_records: &[PublicRegistryCapabilityRecord],
) -> Result<RegisteredBundle, CliError> {
    load_registered_bundle_with_policy(manifest_path, public_records, true)
}

fn load_registered_bundle_with_policy(
    manifest_path: &Path,
    public_records: &[PublicRegistryCapabilityRecord],
    reject_local_public_scope: bool,
) -> Result<RegisteredBundle, CliError> {
    let bundle = load_registry_bundle(manifest_path).map_err(|failure| {
        let msg = failure.errors[0].message.clone();
        CliError::IoError(msg)
    })?;

    let evidence = enforce_bundle_scope(&bundle, public_records, reject_local_public_scope)?;

    let mut capability_registry = CapabilityRegistry::new();
    let mut event_registry = EventRegistry::new();
    let mut workflow_registry = WorkflowRegistry::new();
    let mut capability_records = Vec::new();
    let mut event_records = Vec::new();
    let mut workflow_records = Vec::new();

    register_reference_connectors(&mut capability_registry, &bundle)?;

    for event in &bundle.events {
        let outcome = event_registry
            .register(EventRegistration {
                scope: bundle.scope,
                contract: event.contract.clone(),
                contract_path: event.path.display().to_string(),
                registered_at: bundle_registered_at(&bundle),
                governing_spec: "011-event-registry".to_string(),
                validator_version: env!("CARGO_PKG_VERSION").to_string(),
            })
            .map_err(|f| CliError::RegistrationConflict(render_event_registry_failure(f)))?;
        event_records.push(format!("{}@{}", outcome.record.id, outcome.record.version));
    }

    let mut gate_violations = Vec::new();
    for capability in &bundle.capabilities {
        for referenced in capability
            .contract
            .emits
            .iter()
            .chain(capability.contract.consumes.iter())
        {
            let exists = event_registry
                .find_exact(
                    LookupScope::PreferPrivate,
                    &referenced.event_id,
                    &referenced.version,
                )
                .is_some();
            if !exists {
                gate_violations.push(ViolationRecord::new(
                    "unresolved_event_reference",
                    capability.path.display().to_string(),
                    format!(
                        "capability references missing event {}@{}",
                        referenced.event_id, referenced.version
                    ),
                ));
            }
        }
    }

    if !gate_violations.is_empty() {
        return Err(CliError::ValidationFailed(render_violation_records(
            "registration-time contractual enforcement gate failed",
            &gate_violations,
        )));
    }

    for capability in &bundle.capabilities {
        let request = build_capability_registration(&bundle, capability)?;
        let outcome = capability_registry.register(request).map_err(|f| {
            let msg = render_registry_failure(f.clone());
            map_registry_failure(&f, msg)
        })?;
        capability_records.push(format_capability_record(
            &outcome.record.id,
            &outcome.record.version,
            outcome.record.implementation_kind,
        ));
    }

    for workflow in &bundle.workflows {
        let outcome = workflow_registry
            .register(
                &capability_registry,
                WorkflowRegistration {
                    scope: bundle.scope,
                    definition: workflow.definition.clone(),
                    workflow_path: workflow.path.display().to_string(),
                    registered_at: bundle_registered_at(&bundle),
                    validator_version: env!("CARGO_PKG_VERSION").to_string(),
                },
            )
            .map_err(|f| CliError::ValidationFailed(render_workflow_failure(f)))?;
        workflow_records.push(format!("{}@{}", outcome.record.id, outcome.record.version));
    }

    Ok(RegisteredBundle {
        bundle,
        capability_registry,
        event_registry,
        workflow_registry,
        capability_records,
        event_records,
        workflow_records,
        evidence,
    })
}

fn enforce_bundle_scope(
    bundle: &RegistryBundle,
    public_records: &[PublicRegistryCapabilityRecord],
    reject_local_public_scope: bool,
) -> Result<Vec<BundleRegistrationEvidence>, CliError> {
    if reject_local_public_scope && bundle.scope == RegistryScope::Public {
        return Err(CliError::ValidationFailed(
            "local_public_scope_rejected: local bundle registration cannot populate the public registry tier; use scope: private for local testing or traverse-cli capability publish for governed publication ($.scope)".to_string(),
        ));
    }

    Ok(bundle
        .capabilities
        .iter()
        .filter(|capability| {
            public_records.iter().any(|public| {
                public.id == capability.manifest.id
                    && public.version == capability.manifest.version
            })
        })
        .map(|capability| BundleRegistrationEvidence {
            code: "private_shadows_synced_public",
            path: capability.path.display().to_string(),
            message: format!(
                "private capability {}@{} overrides the synced public record for prefer-private lookup",
                capability.manifest.id, capability.manifest.version
            ),
            capability_id: capability.manifest.id.clone(),
            capability_version: capability.manifest.version.clone(),
            lookup_scope: "prefer_private",
        })
        .collect())
}

fn register_reference_connectors(
    capability_registry: &mut CapabilityRegistry,
    bundle: &RegistryBundle,
) -> Result<(), CliError> {
    for connector in reference_connector_contracts() {
        capability_registry
            .register_connector(ConnectorRegistration {
                scope: RegistryScope::Public,
                contract_path: format!(
                    "contracts/connectors/{}/connector_contract.json",
                    connector.connector_id
                ),
                contract: connector,
                registered_at: bundle_registered_at(bundle),
                governing_spec: "039-connector-plugin-architecture".to_string(),
                validator_version: env!("CARGO_PKG_VERSION").to_string(),
            })
            .map_err(|f| CliError::RegistrationConflict(render_registry_failure(f)))?;
    }
    Ok(())
}

fn render_violation_records(header: &str, violations: &[ViolationRecord]) -> String {
    let mut lines = Vec::new();
    lines.push(header.to_string());
    let mut sorted = violations.to_vec();
    sorted.sort_by(|a, b| {
        (a.path.as_str(), a.violation_code.as_str())
            .cmp(&(b.path.as_str(), b.violation_code.as_str()))
    });
    for v in sorted {
        lines.push(format!(
            "- [{}] {}: {}",
            v.violation_code, v.path, v.message
        ));
    }
    lines.join("\n")
}

fn map_registry_failure(failure: &traverse_registry::RegistryFailure, msg: String) -> CliError {
    use traverse_registry::RegistryErrorCode;
    if failure.errors.iter().any(|e| {
        matches!(
            e.code,
            RegistryErrorCode::ImmutableVersionConflict
                | RegistryErrorCode::DuplicateItem
                | RegistryErrorCode::ArtifactConflict
        )
    }) {
        CliError::RegistrationConflict(msg)
    } else if failure
        .errors
        .iter()
        .any(|e| matches!(e.code, RegistryErrorCode::ContractValidationFailed))
    {
        CliError::ValidationFailed(msg)
    } else {
        CliError::IoError(msg)
    }
}

fn load_runtime_request(request_path: &Path) -> Result<RuntimeRequest, CliError> {
    let contents = read_text_file(request_path, "runtime request")?;
    parse_runtime_request(&contents).map_err(|error| {
        CliError::ValidationFailed(format!(
            "failed to parse runtime request {}: {error}",
            request_path.display()
        ))
    })
}

fn parse_capability_registration_envelope(
    raw_contract: &str,
    path: &Path,
) -> Result<Value, CliError> {
    serde_json::from_str::<Value>(raw_contract).map_err(|error| {
        CliError::ValidationFailed(format!(
            "failed to parse capability registration metadata {}: {error}",
            path.display()
        ))
    })
}

fn derive_implementation_kind(composability_value: Option<&Value>) -> ImplementationKind {
    match composability_value
        .and_then(|composability| composability.get("implementation_kind"))
        .and_then(Value::as_str)
    {
        Some("workflow") => ImplementationKind::Workflow,
        _ => ImplementationKind::Executable,
    }
}

fn derive_workflow_ref(
    composability_value: Option<&Value>,
) -> Result<Option<WorkflowReference>, CliError> {
    composability_value
        .and_then(|composability| composability.get("workflow_ref"))
        .map(parse_workflow_ref)
        .transpose()
}

fn derive_composability_metadata(
    implementation_kind: ImplementationKind,
    workflow_ref: Option<&WorkflowReference>,
    capability: &traverse_registry::CapabilityBundleArtifact,
) -> Result<ComposabilityMetadata, CliError> {
    let requires = capability
        .contract
        .consumes
        .iter()
        .map(|event| event.event_id.clone())
        .collect();

    match implementation_kind {
        ImplementationKind::Workflow => {
            if workflow_ref.is_none() {
                return Err(CliError::ValidationFailed(format!(
                    "workflow-backed capability {} must declare workflow_ref",
                    capability.contract.id
                )));
            }
            Ok(ComposabilityMetadata {
                kind: CompositionKind::Composite,
                patterns: vec![CompositionPattern::Sequential],
                provides: vec![capability.contract.id.clone()],
                requires,
            })
        }
        ImplementationKind::Executable => Ok(ComposabilityMetadata {
            kind: CompositionKind::Atomic,
            patterns: vec![CompositionPattern::Sequential],
            provides: vec![capability.contract.id.clone()],
            requires,
        }),
    }
}

fn build_capability_artifact(
    bundle: &RegistryBundle,
    capability: &traverse_registry::CapabilityBundleArtifact,
    implementation_kind: ImplementationKind,
    workflow_ref: Option<WorkflowReference>,
) -> CapabilityArtifactRecord {
    CapabilityArtifactRecord {
        artifact_ref: format!(
            "bundle:{}:{}:{}",
            bundle.bundle_id, capability.contract.id, capability.contract.version
        ),
        implementation_kind,
        source: SourceReference {
            kind: SourceKind::Local,
            location: capability.path.display().to_string(),
        },
        binary: match implementation_kind {
            ImplementationKind::Executable => Some(BinaryReference {
                format: BinaryFormat::Wasm,
                location: format!(
                    "bundled://{}/{}/module.wasm",
                    capability.contract.id, capability.contract.version
                ),
                signature: None,
            }),
            ImplementationKind::Workflow => None,
        },
        workflow_ref,
        digests: ArtifactDigests {
            source_digest: format!(
                "source:{}:{}",
                capability.contract.id, capability.contract.version
            ),
            binary_digest: match implementation_kind {
                ImplementationKind::Executable => Some(format!(
                    "binary:{}:{}",
                    capability.contract.id, capability.contract.version
                )),
                ImplementationKind::Workflow => None,
            },
        },
        provenance: RegistryProvenance {
            source: provenance_source_label(&capability.contract.provenance.source),
            author: capability.contract.provenance.author.clone(),
            created_at: capability.contract.provenance.created_at.clone(),
        },
    }
}

fn bundle_registered_at(bundle: &RegistryBundle) -> String {
    format!("bundle:{}@{}", bundle.bundle_id, bundle.version)
}

fn parse_workflow_ref(value: &Value) -> Result<WorkflowReference, CliError> {
    let workflow_id = value
        .get("workflow_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            CliError::ValidationFailed("workflow_ref.workflow_id must be a string".to_string())
        })?;
    let workflow_version = value
        .get("workflow_version")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            CliError::ValidationFailed("workflow_ref.workflow_version must be a string".to_string())
        })?;
    Ok(WorkflowReference {
        workflow_id: workflow_id.to_string(),
        workflow_version: workflow_version.to_string(),
    })
}

fn provenance_source_label(source: &traverse_contracts::ProvenanceSource) -> String {
    match source {
        traverse_contracts::ProvenanceSource::Greenfield => "greenfield",
        traverse_contracts::ProvenanceSource::BrownfieldExtracted => "brownfield-extracted",
        traverse_contracts::ProvenanceSource::AiGenerated => "ai-generated",
        traverse_contracts::ProvenanceSource::AiAssisted => "ai-assisted",
    }
    .to_string()
}

fn canonical_expedition_bundle_path() -> PathBuf {
    repo_root().join("examples/expedition/registry-bundle/manifest.json")
}

/// Manifest paths for the five atomic expedition-planning capabilities' real
/// WASM agent packages (spec 916/921). Each is registered individually via
/// [`load_capability_package`] so its `CapabilityRegistration` carries a real
/// binary path and digest — the canonical expedition registry bundle's own
/// capability entries only ever carry a fabricated `bundled://...` artifact
/// reference (see `build_capability_artifact`), which `ArtifactRouter` cannot
/// resolve.
fn expedition_atomic_capability_manifest_paths() -> [PathBuf; 5] {
    let root = repo_root();
    [
        root.join("examples/capabilities/capture-expedition-objective-agent/manifest.json"),
        root.join("examples/capabilities/expedition-intent-agent/manifest.json"),
        root.join("examples/capabilities/assess-conditions-summary-agent/manifest.json"),
        root.join("examples/capabilities/team-readiness-agent/manifest.json"),
        root.join("examples/capabilities/assemble-expedition-plan-agent/manifest.json"),
    ]
}

/// Builds a `traverse-cli expedition execute` runtime executing the real
/// registered WASM artifact for every expedition-planning capability, via
/// the same `ArtifactRouter` execution path `capability-package execute` and
/// `serve` use. The composite `plan-expedition` capability (workflow kind,
/// no binary of its own) is registered from the canonical expedition
/// registry bundle; the five atomic capabilities are registered from their
/// real agent packages so their artifact info is genuine, not fabricated.
fn build_expedition_runtime() -> Result<Runtime<ArtifactRouter>, CliError> {
    let bundle_path = canonical_expedition_bundle_path();
    let bundle = load_registry_bundle(&bundle_path)
        .map_err(|failure| CliError::IoError(failure.errors[0].message.clone()))?;

    let mut capability_registry = CapabilityRegistry::new();
    let composite = bundle
        .capabilities
        .iter()
        .find(|capability| capability.contract.id == "expedition.planning.plan-expedition")
        .ok_or_else(|| {
            CliError::IoError(
                "expedition registry bundle is missing the plan-expedition capability".to_string(),
            )
        })?;
    let composite_registration = build_capability_registration(&bundle, composite)?;
    capability_registry
        .register(composite_registration)
        .map_err(|f| CliError::RegistrationConflict(render_registry_failure(f)))?;

    for manifest_path in expedition_atomic_capability_manifest_paths() {
        let package = load_capability_package(&manifest_path).map_err(CliError::IoError)?;
        capability_registry
            .register(package.capability_registration())
            .map_err(|f| CliError::RegistrationConflict(render_registry_failure(f)))?;
    }

    let mut workflow_registry = WorkflowRegistry::new();
    for workflow in &bundle.workflows {
        workflow_registry
            .register(
                &capability_registry,
                WorkflowRegistration {
                    scope: bundle.scope,
                    definition: workflow.definition.clone(),
                    workflow_path: workflow.path.display().to_string(),
                    registered_at: bundle_registered_at(&bundle),
                    validator_version: env!("CARGO_PKG_VERSION").to_string(),
                },
            )
            .map_err(|f| CliError::ValidationFailed(render_workflow_failure(f)))?;
    }

    let runtime = Runtime::new(
        capability_registry,
        ArtifactRouter::new().map_err(|error| CliError::ExecutionFailed(error.message))?,
    )
    .with_workflow_registry(workflow_registry)
    .with_security_config(traverse_runtime::security::RuntimeSecurityConfig::development());
    Ok(runtime)
}

fn execute_expedition_outcome(request_path: &Path) -> Result<RuntimeExecutionOutcome, CliError> {
    let request = load_runtime_request(request_path)?;
    let runtime = build_expedition_runtime()?;
    Ok(runtime.execute(request))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn format_capability_record(
    id: &str,
    version: &str,
    implementation_kind: ImplementationKind,
) -> String {
    let kind = match implementation_kind {
        ImplementationKind::Executable => "executable",
        ImplementationKind::Workflow => "workflow",
    };
    format!("{id}@{version} ({kind})")
}

fn render_registry_failure(failure: traverse_registry::RegistryFailure) -> String {
    failure
        .errors
        .into_iter()
        .map(|error| format!("{} at {}", error.message, error.target))
        .collect::<Vec<_>>()
        .join("; ")
}

fn render_event_registry_failure(failure: traverse_registry::EventRegistryFailure) -> String {
    failure
        .errors
        .into_iter()
        .map(|error| format!("{} at {}", error.message, error.target))
        .collect::<Vec<_>>()
        .join("; ")
}

fn render_workflow_failure(failure: traverse_registry::WorkflowFailure) -> String {
    failure
        .errors
        .into_iter()
        .map(|error| format!("{} at {}", error.message, error.path))
        .collect::<Vec<_>>()
        .join("; ")
}

fn render_runtime_execution_failure(outcome: &RuntimeExecutionOutcome) -> String {
    match &outcome.result.error {
        Some(error) => format!("runtime execution failed: {}", error.message),
        None => "runtime execution failed".to_string(),
    }
}

fn execute_capture_expedition_objective(input: &Value) -> Result<Value, LocalExecutionFailure> {
    let map = input_object(input)?;
    let destination = required_value(map, "destination")?;
    let target_window = required_value(map, "target_window")?;
    let preferences = required_value(map, "preferences")?;
    let notes = required_value(map, "notes")?;
    let objective_id = format!("objective-{}", slug(required_string(map, "destination")?));
    let objective = serde_json::json!({
        "objective_id": objective_id,
        "destination": destination.clone(),
        "target_window": target_window.clone(),
        "preferences": preferences.clone(),
        "notes": notes.clone()
    });

    Ok(serde_json::json!({
        "objective_id": objective_id,
        "destination": destination.clone(),
        "target_window": target_window.clone(),
        "preferences": preferences.clone(),
        "notes": notes.clone(),
        "objective": objective,
        "emitted_events": [event_ref("expedition.planning.expedition-objective-captured")]
    }))
}

fn execute_interpret_expedition_intent(input: &Value) -> Result<Value, LocalExecutionFailure> {
    let map = input_object(input)?;
    let objective = required_object(map, "objective")?;
    let objective_id = required_string(objective, "objective_id")?;
    let preferences = required_object(objective, "preferences")?;
    let style = required_string(preferences, "style")?;
    let priority = required_string(preferences, "priority")?;
    let planning_intent = required_string(map, "planning_intent")?;
    let interpreted_intent = serde_json::json!({
        "intent_id": format!("intent-{objective_id}"),
        "objective_id": objective_id,
        "route_preferences": [style, priority],
        "constraints": [format!("priority:{priority}")],
        "assumptions": [planning_intent],
        "confidence": 0.87
    });

    Ok(serde_json::json!({
        "intent_id": format!("intent-{objective_id}"),
        "objective_id": objective_id,
        "route_preferences": [style, priority],
        "constraints": [format!("priority:{priority}")],
        "assumptions": [planning_intent],
        "confidence": 0.87,
        "interpreted_intent": interpreted_intent,
        "emitted_events": [event_ref("expedition.planning.expedition-intent-interpreted")]
    }))
}

fn execute_assess_conditions_summary(input: &Value) -> Result<Value, LocalExecutionFailure> {
    let map = input_object(input)?;
    let objective = required_object(map, "objective")?;
    let objective_id = required_string(objective, "objective_id")?;
    let destination = required_string(objective, "destination")?;
    let interpreted = required_object(map, "interpreted_intent")?;
    let route_preferences = required_string_array(interpreted, "route_preferences")?;
    let conditions_summary = serde_json::json!({
        "conditions_summary_id": format!("conditions-{objective_id}"),
        "objective_id": objective_id,
        "overall_rating": "watchful",
        "key_findings": [format!("stable morning window for {destination}"), format!("preferred style: {}", route_preferences.first().cloned().unwrap_or_else(|| "conservative".to_string()))],
        "blocking_concerns": []
    });

    Ok(serde_json::json!({
        "conditions_summary_id": format!("conditions-{objective_id}"),
        "objective_id": objective_id,
        "overall_rating": "watchful",
        "key_findings": [format!("stable morning window for {destination}"), format!("preferred style: {}", route_preferences.first().cloned().unwrap_or_else(|| "conservative".to_string()))],
        "blocking_concerns": [],
        "conditions_summary": conditions_summary,
        "emitted_events": [event_ref("expedition.planning.conditions-summary-assessed")]
    }))
}

fn execute_validate_team_readiness(input: &Value) -> Result<Value, LocalExecutionFailure> {
    let map = input_object(input)?;
    let objective = required_object(map, "objective")?;
    let objective_id = required_string(objective, "objective_id")?;
    let team_profile = required_object(map, "team_profile")?;
    let equipment_ready = required_bool(team_profile, "equipment_ready")?;
    let status = if equipment_ready {
        "ready"
    } else {
        "needs_action"
    };
    let required_actions = if equipment_ready {
        Vec::<String>::new()
    } else {
        vec!["complete equipment verification".to_string()]
    };
    let readiness_result = serde_json::json!({
        "readiness_result_id": format!("readiness-{objective_id}"),
        "objective_id": objective_id,
        "status": status,
        "reasons": ["team profile satisfies baseline expedition requirements"],
        "required_actions": required_actions.clone()
    });

    Ok(serde_json::json!({
        "readiness_result_id": format!("readiness-{objective_id}"),
        "objective_id": objective_id,
        "status": status,
        "reasons": ["team profile satisfies baseline expedition requirements"],
        "required_actions": required_actions,
        "readiness_result": readiness_result,
        "emitted_events": [event_ref("expedition.planning.team-readiness-validated")]
    }))
}

fn execute_assemble_expedition_plan(input: &Value) -> Result<Value, LocalExecutionFailure> {
    let map = input_object(input)?;
    let objective = required_object(map, "objective")?;
    let objective_id = required_string(objective, "objective_id")?;
    let interpreted = required_object(map, "interpreted_intent")?;
    let route_preferences = required_string_array(interpreted, "route_preferences")?;
    let constraints = required_string_array(interpreted, "constraints")?;
    let readiness = required_object(map, "readiness_result")?;
    let readiness_status = required_string(readiness, "status")?;
    let readiness_reasons = required_string_array(readiness, "reasons")?;
    let required_actions = required_string_array(readiness, "required_actions")?;
    let route_style = route_preferences
        .first()
        .cloned()
        .unwrap_or_else(|| "conservative-alpine-push".to_string());

    let mut readiness_notes = readiness_reasons;
    readiness_notes.extend(required_actions);

    Ok(serde_json::json!({
        "plan_id": format!("plan-{objective_id}"),
        "objective_id": objective_id,
        "status": if readiness_status == "ready" { "ready" } else { "requires_attention" },
        "recommended_route_style": route_style,
        "key_steps": [
            "depart before sunrise",
            "reassess winds at mid-route checkpoint",
            "apply conservative turnaround time"
        ],
        "constraints": constraints,
        "readiness_notes": readiness_notes,
        "summary": "Proceed with a conservative same-day ascent plan under a limited morning weather window.",
        "emitted_events": [event_ref("expedition.planning.expedition-plan-assembled")]
    }))
}

const STARTER_NOTE_MAX_CHARS: usize = 2000;

fn execute_traverse_starter_validate(input: &Value) -> Result<Value, LocalExecutionFailure> {
    let map = input_object(input)?;
    let note = required_string(map, "note")?;
    let mut issues = Vec::new();
    if note.trim().is_empty() {
        issues.push("note must not be empty".to_string());
    }
    if note.chars().count() > STARTER_NOTE_MAX_CHARS {
        issues.push(format!(
            "note must be at most {STARTER_NOTE_MAX_CHARS} characters"
        ));
    }

    Ok(serde_json::json!({
        "valid": issues.is_empty(),
        "issues": issues
    }))
}

fn execute_traverse_starter_summarize(input: &Value) -> Result<Value, LocalExecutionFailure> {
    let map = input_object(input)?;
    let title = required_string(map, "title")?;
    let note_type = required_string(map, "noteType")?;
    let suggested_next_action = required_string(map, "suggestedNextAction")?;
    let tags = required_string_array(map, "tags")?;
    let tag_list = if tags.is_empty() {
        "none".to_string()
    } else {
        tags.join(", ")
    };
    let summary =
        format!("{title} ({note_type}) - tags: {tag_list}; next action: {suggested_next_action}");
    let word_count = summary.split_whitespace().count();

    Ok(serde_json::json!({
        "summary": summary,
        "wordCount": word_count
    }))
}

fn execute_traverse_starter_process(input: &Value) -> Result<Value, LocalExecutionFailure> {
    let map = input_object(input)?;
    let note = required_string(map, "note")?;
    let trimmed = note.trim();
    let title_words = trimmed.split_whitespace().take(5).collect::<Vec<_>>();
    let title = if title_words.is_empty() {
        "Untitled note".to_string()
    } else {
        title_words.join(" ")
    };
    let tags = derive_starter_tags(trimmed);
    let note_type =
        if contains_ascii_word(trimmed, "project") || contains_ascii_word(trimmed, "app") {
            "project"
        } else if trimmed.len() > 80 {
            "permanent"
        } else {
            "fleeting"
        };
    let suggested_next_action = match note_type {
        "project" => "expand",
        "permanent" => "review",
        _ => "archive",
    };

    Ok(serde_json::json!({
        "title": title,
        "tags": tags,
        "noteType": note_type,
        "suggestedNextAction": suggested_next_action,
        "status": "complete"
    }))
}

fn execute_meeting_notes_process(input: &Value) -> Result<Value, LocalExecutionFailure> {
    let map = input_object(input)?;
    let transcript = required_string(map, "transcript")?;
    let trimmed = transcript.trim();
    let mut action_items = Vec::new();
    let mut decisions = Vec::new();
    let mut follow_ups = Vec::new();

    for line in trimmed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let lower = line.to_ascii_lowercase();
        if lower.contains("action:")
            || lower.contains("todo:")
            || lower.contains("will do")
            || lower.contains("to do")
            || line.contains('@')
        {
            action_items.push(serde_json::json!({
                "task": clean_meeting_marker(line),
                "owner": meeting_owner(line),
                "due": meeting_due(line)
            }));
        }
        if lower.contains("decided:")
            || lower.contains("agreed:")
            || lower.contains("we will")
            || lower.contains("resolution:")
        {
            decisions.push(serde_json::json!({
                "text": clean_meeting_marker(line),
                "made_by": meeting_owner(line)
            }));
        }
        if lower.contains("follow up")
            || lower.contains("check in")
            || lower.contains("revisit")
            || lower.contains("next steps")
        {
            follow_ups.push(Value::String(clean_meeting_marker(line)));
        }
    }

    Ok(serde_json::json!({
        "action_items": action_items,
        "decisions": decisions,
        "follow_ups": follow_ups,
        "summary": meeting_summary(trimmed)
    }))
}

fn clean_meeting_marker(line: &str) -> String {
    let trimmed = line.trim();
    for marker in ["action:", "todo:", "decided:", "agreed:", "resolution:"] {
        if trimmed
            .get(..marker.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(marker))
        {
            return trimmed[marker.len()..].trim().to_string();
        }
    }
    trimmed.to_string()
}

fn meeting_owner(line: &str) -> Value {
    if let Some(owner) = line
        .split_whitespace()
        .find_map(|word| word.strip_prefix('@'))
        .map(|word| word.trim_matches(|ch: char| !ch.is_ascii_alphanumeric()))
        .filter(|word| !word.is_empty())
    {
        return Value::String(owner.to_string());
    }

    let words = line.split_whitespace().collect::<Vec<_>>();
    for pair in words.windows(2) {
        if pair[0].eq_ignore_ascii_case("by") {
            let owner = pair[1].trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
            if !owner.is_empty() {
                return Value::String(owner.to_string());
            }
        }
    }
    Value::Null
}

fn meeting_due(line: &str) -> Value {
    let words = line.split_whitespace().collect::<Vec<_>>();
    for pair in words.windows(2) {
        if pair[0].eq_ignore_ascii_case("by")
            || pair[0].eq_ignore_ascii_case("before")
            || pair[0].eq_ignore_ascii_case("due")
        {
            let due = pair[1].trim_matches(|ch: char| ch == '.' || ch == ',' || ch == ';');
            if looks_like_due_token(due) {
                return Value::String(due.to_string());
            }
        }
    }
    Value::Null
}

fn looks_like_due_token(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let lower = token.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "today"
            | "tomorrow"
            | "monday"
            | "tuesday"
            | "wednesday"
            | "thursday"
            | "friday"
            | "saturday"
            | "sunday"
    ) || token.chars().any(|ch| ch.is_ascii_digit())
}

fn meeting_summary(transcript: &str) -> String {
    if transcript.is_empty() {
        return String::new();
    }
    let first_paragraph = transcript
        .split("\n\n")
        .find(|paragraph| !paragraph.trim().is_empty())
        .unwrap_or(transcript)
        .trim();
    first_paragraph.chars().take(280).collect()
}

fn derive_starter_tags(note: &str) -> Vec<String> {
    let mut tags = Vec::new();
    for word in note.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        let normalized = word.to_ascii_lowercase();
        if normalized.len() < 4 || tags.iter().any(|tag| tag == &normalized) {
            continue;
        }
        tags.push(normalized);
        if tags.len() == 3 {
            break;
        }
    }
    if tags.is_empty() {
        tags.push("note".to_string());
    }
    tags
}

fn contains_ascii_word(text: &str, expected: &str) -> bool {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|word| word.eq_ignore_ascii_case(expected))
}

fn event_ref(event_id: &str) -> Value {
    serde_json::json!({
        "event_id": event_id,
        "version": "1.0.0"
    })
}

fn input_object(value: &Value) -> Result<&serde_json::Map<String, Value>, LocalExecutionFailure> {
    value
        .as_object()
        .ok_or_else(|| executor_failure("executor input must be an object"))
}

fn required_object<'a>(
    map: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a serde_json::Map<String, Value>, LocalExecutionFailure> {
    map.get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| executor_failure(&format!("missing object field: {key}")))
}

fn required_value<'a>(
    map: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a Value, LocalExecutionFailure> {
    map.get(key)
        .ok_or_else(|| executor_failure(&format!("missing field: {key}")))
}

fn required_string<'a>(
    map: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a str, LocalExecutionFailure> {
    map.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| executor_failure(&format!("missing string field: {key}")))
}

fn required_bool(
    map: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<bool, LocalExecutionFailure> {
    map.get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| executor_failure(&format!("missing boolean field: {key}")))
}

fn required_string_array(
    map: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Vec<String>, LocalExecutionFailure> {
    let items = map
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| executor_failure(&format!("missing string array field: {key}")))?;

    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(ToString::to_string)
                .ok_or_else(|| executor_failure(&format!("invalid string array field: {key}")))
        })
        .collect()
}

fn executor_failure(message: &str) -> LocalExecutionFailure {
    LocalExecutionFailure {
        code: LocalExecutionFailureCode::ExecutionFailed,
        message: message.to_string(),
    }
}

fn slug(value: &str) -> String {
    let mut slug = String::new();
    for component in Path::new(value).components() {
        if let Component::Normal(part) = component {
            let part = part.to_string_lossy();
            for ch in part.chars() {
                if ch.is_ascii_alphanumeric() {
                    slug.push(ch.to_ascii_lowercase());
                }
            }
        }
    }
    if slug.is_empty() {
        "expedition".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::{
        AppValidationError, ArtifactRouter, CapabilityPublishRequest, CapabilityRegistry, CliError,
        Command, DEFAULT_PUBLIC_REGISTRY_SOURCE, ExpeditionExampleExecutor, FetchedRegistryIndex,
        PublishCommandOutput, PublishProcessRunner, RealPublishProcessRunner, RegistryIndexFetcher,
        RegistrySyncError, Runtime, RuntimeResultStatus, SUPPORTED_HOST_ABI_VERSION,
        app_activate_at, app_activation_state_path, app_new_at, app_register_at,
        app_registration_state_path, app_validate, app_validate_at,
        canonical_expedition_bundle_path, capability_new_at, capability_publish_at, component_new,
        curl_text, discover_capabilities, enforce_contract_surface_coverage,
        enforce_persona_refs_resolve, ensure_clean_registry_checkout, execute_capability_package,
        execute_expedition, execute_traverse_starter_process, execute_traverse_starter_summarize,
        execute_traverse_starter_validate, format_capability_package_execution_summary,
        help_expedition_execute, help_serve, inspect_bundle, inspect_capability,
        inspect_capability_package, inspect_event, inspect_trace, latest_index_release_asset,
        load_capability_package, load_registered_bundle,
        load_registered_bundle_with_public_records, load_runtime_request, parse_command,
        publish_file_sha256_digest, register_bundle, register_generated_app_bundle,
        registry_record_order, registry_sync_at, registry_sync_default_or_override,
        registry_sync_failure_json, reject_private_contract_scope, run_command, sha256_hex,
        surface_coverage_gap_messages, telemetry, uncovered_action_enum_values,
        unresolved_persona_refs, use_case_smoke_coverage_gaps,
        use_case_smoke_coverage_gaps_for_package, validate_component_risk_policy_for_cli,
        validate_registry_path_segment,
    };
    use crate::capability_packages::fnv1a64;
    use serde_json::Value;
    use std::cell::RefCell;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command as ProcessCommand;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};
    use traverse_contracts::parse_contract;
    use traverse_contracts::{UsageEvent, UsageEventKind, UsageTelemetrySink};
    use traverse_registry::{
        PublicRegistryCapabilityRecord, PublicRegistryIndex, load_application_bundle_manifest,
        load_synced_public_registry_state, write_synced_public_registry_state,
    };

    #[test]
    fn parse_command_accepts_supported_inspect_commands() {
        let bundle = vec![
            "traverse-cli".to_string(),
            "bundle".to_string(),
            "inspect".to_string(),
            "examples/expedition/registry-bundle/manifest.json".to_string(),
        ];
        let bundle_register = vec![
            "traverse-cli".to_string(),
            "bundle".to_string(),
            "register".to_string(),
            "examples/expedition/registry-bundle/manifest.json".to_string(),
        ];
        let capability_package_inspect = vec![
            "traverse-cli".to_string(),
            "capability-package".to_string(),
            "inspect".to_string(),
            "examples/capabilities/expedition-intent-agent/manifest.json".to_string(),
        ];
        let capability_package_execute = vec![
            "traverse-cli".to_string(),
            "capability-package".to_string(),
            "execute".to_string(),
            "examples/capabilities/expedition-intent-agent/manifest.json".to_string(),
            "examples/capabilities/runtime-requests/interpret-expedition-intent.json".to_string(),
        ];
        let wasm_abi_verify = vec![
            "traverse-cli".to_string(),
            "wasm".to_string(),
            "abi".to_string(),
            "verify".to_string(),
            "examples/hello-world/say-hello-agent/artifacts/say-hello-agent.wasm".to_string(),
        ];
        let artifact_verify = vec![
            "traverse-cli".to_string(),
            "artifact".to_string(),
            "verify".to_string(),
            "target/release/traverse-cli".to_string(),
        ];
        let expedition_execute = vec![
            "traverse-cli".to_string(),
            "expedition".to_string(),
            "execute".to_string(),
            "examples/expedition/runtime-requests/plan-expedition.json".to_string(),
        ];
        let event = vec![
            "traverse-cli".to_string(),
            "event".to_string(),
            "inspect".to_string(),
            "contracts/examples/expedition/events/expedition-objective-captured/contract.json"
                .to_string(),
        ];
        let trace = vec![
            "traverse-cli".to_string(),
            "trace".to_string(),
            "inspect".to_string(),
            "/tmp/plan-expedition-trace.json".to_string(),
        ];
        let workflow = vec![
            "traverse-cli".to_string(),
            "workflow".to_string(),
            "inspect".to_string(),
            "workflows/examples/expedition/plan-expedition/workflow.json".to_string(),
        ];
        let expedition_execute_with_trace = vec![
            "traverse-cli".to_string(),
            "expedition".to_string(),
            "execute".to_string(),
            "examples/expedition/runtime-requests/plan-expedition.json".to_string(),
            "--trace-out".to_string(),
            "/tmp/plan-expedition-trace.json".to_string(),
        ];
        let app_new = vec![
            "traverse-cli".to_string(),
            "app".to_string(),
            "new".to_string(),
            "youaskm3".to_string(),
        ];
        let component_new = vec![
            "traverse-cli".to_string(),
            "component".to_string(),
            "new".to_string(),
            "knowledge.retrieve".to_string(),
        ];
        assert!(parse_command(&bundle).is_ok());
        assert!(parse_command(&bundle_register).is_ok());
        assert!(parse_command(&capability_package_inspect).is_ok());
        assert!(parse_command(&capability_package_execute).is_ok());
        assert!(parse_command(&wasm_abi_verify).is_ok());
        assert!(parse_command(&artifact_verify).is_ok());
        assert!(parse_command(&expedition_execute).is_ok());
        assert!(parse_command(&expedition_execute_with_trace).is_ok());
        assert!(parse_command(&event).is_ok());
        assert!(parse_command(&trace).is_ok());
        assert!(parse_command(&workflow).is_ok());
        assert!(parse_command(&app_new).is_ok());
        assert!(parse_command(&component_new).is_ok());
    }

    #[test]
    fn parse_app_new_accepts_register_workspace_flags() {
        let args = vec![
            "traverse-cli".to_string(),
            "app".to_string(),
            "new".to_string(),
            "youaskm3".to_string(),
            "--register".to_string(),
            "--workspace".to_string(),
            "local-dev".to_string(),
        ];

        let command = parse_command(&args).expect("app new should parse");

        match command {
            Command::AppNew {
                app_id,
                register,
                workspace_id,
            } => {
                assert_eq!(app_id, "youaskm3");
                assert!(register);
                assert_eq!(workspace_id.as_deref(), Some("local-dev"));
            }
            other => assert!(matches!(other, Command::AppNew { .. })),
        }
    }

    #[test]
    fn parse_app_validate_requires_manifest_and_json_flags() {
        let args = vec![
            "traverse-cli".to_string(),
            "app".to_string(),
            "validate".to_string(),
            "--manifest".to_string(),
            "examples/applications/expedition-readiness/app.manifest.json".to_string(),
            "--json".to_string(),
        ];

        let command = parse_command(&args).expect("app validate should parse");

        match command {
            Command::AppValidate {
                manifest_path,
                json_output,
                ..
            } => {
                assert_eq!(
                    manifest_path,
                    PathBuf::from("examples/applications/expedition-readiness/app.manifest.json")
                );
                assert!(json_output);
            }
            other => assert!(matches!(other, Command::AppValidate { .. })),
        }

        let missing_json = vec![
            "traverse-cli".to_string(),
            "app".to_string(),
            "validate".to_string(),
            "--manifest".to_string(),
            "examples/applications/expedition-readiness/app.manifest.json".to_string(),
        ];
        assert!(parse_command(&missing_json).is_err());
    }

    #[test]
    fn parse_app_register_requires_manifest_workspace_and_json_flags() {
        let args = vec![
            "traverse-cli".to_string(),
            "app".to_string(),
            "register".to_string(),
            "--manifest".to_string(),
            "examples/applications/expedition-readiness/app.manifest.json".to_string(),
            "--workspace".to_string(),
            "local-dev".to_string(),
            "--json".to_string(),
        ];

        let command = parse_command(&args).expect("app register should parse");

        match command {
            Command::AppRegister {
                manifest_path,
                workspace_id,
                json_output,
            } => {
                assert_eq!(
                    manifest_path,
                    PathBuf::from("examples/applications/expedition-readiness/app.manifest.json")
                );
                assert_eq!(workspace_id, "local-dev");
                assert!(json_output);
            }
            other => assert!(matches!(other, Command::AppRegister { .. })),
        }

        let missing_workspace = vec![
            "traverse-cli".to_string(),
            "app".to_string(),
            "register".to_string(),
            "--manifest".to_string(),
            "examples/applications/expedition-readiness/app.manifest.json".to_string(),
            "--json".to_string(),
        ];
        assert!(parse_command(&missing_workspace).is_err());

        let missing_json = vec![
            "traverse-cli".to_string(),
            "app".to_string(),
            "register".to_string(),
            "--manifest".to_string(),
            "examples/applications/expedition-readiness/app.manifest.json".to_string(),
            "--workspace".to_string(),
            "local-dev".to_string(),
        ];
        assert!(parse_command(&missing_json).is_err());
    }

    #[test]
    fn parse_app_activate_requires_all_host_activation_flags() {
        let args = vec![
            "traverse-cli".to_string(),
            "app".to_string(),
            "activate".to_string(),
            "--manifest".to_string(),
            "app.manifest.json".to_string(),
            "--workspace".to_string(),
            "local".to_string(),
            "--host-activation".to_string(),
            "host-activation.json".to_string(),
            "--json".to_string(),
        ];
        let command = parse_command(&args).expect("app activate should parse");
        assert!(matches!(
            command,
            Command::AppActivate {
                manifest_path,
                workspace_id,
                host_activation_path,
                json_output: true,
            } if manifest_path == Path::new("app.manifest.json")
                && workspace_id == "local"
                && host_activation_path == Path::new("host-activation.json")
        ));

        let missing_host_input = vec![
            "traverse-cli".to_string(),
            "app".to_string(),
            "activate".to_string(),
            "--manifest".to_string(),
            "app.manifest.json".to_string(),
            "--workspace".to_string(),
            "local".to_string(),
            "--json".to_string(),
        ];
        assert!(parse_command(&missing_host_input).is_err());
    }

    #[test]
    fn parse_registry_sync_requires_workspace_and_json_flags() {
        let args = vec![
            "traverse-cli".to_string(),
            "registry".to_string(),
            "sync".to_string(),
            "--workspace".to_string(),
            "local-default".to_string(),
            "--json".to_string(),
        ];

        let command = parse_command(&args).expect("registry sync should parse");

        match command {
            Command::RegistrySync {
                workspace_id,
                json_output,
                source_repo,
            } => {
                assert_eq!(workspace_id, "local-default");
                assert!(json_output);
                assert_eq!(source_repo, None);
            }
            other => assert!(matches!(other, Command::RegistrySync { .. })),
        }

        let missing_workspace = vec![
            "traverse-cli".to_string(),
            "registry".to_string(),
            "sync".to_string(),
            "--json".to_string(),
        ];
        assert!(parse_command(&missing_workspace).is_err());

        let missing_json = vec![
            "traverse-cli".to_string(),
            "registry".to_string(),
            "sync".to_string(),
            "--workspace".to_string(),
            "local-default".to_string(),
        ];
        assert!(parse_command(&missing_json).is_err());
    }

    #[test]
    fn parse_registry_sync_accepts_source_repo_override() {
        let args = vec![
            "traverse-cli".to_string(),
            "registry".to_string(),
            "sync".to_string(),
            "--workspace".to_string(),
            "local-default".to_string(),
            "--json".to_string(),
            "--source-repo".to_string(),
            "acme-corp/internal-registry".to_string(),
        ];

        let command = parse_command(&args).expect("registry sync with override should parse");

        match command {
            Command::RegistrySync { source_repo, .. } => {
                assert_eq!(source_repo.as_deref(), Some("acme-corp/internal-registry"));
            }
            other => assert!(matches!(other, Command::RegistrySync { .. })),
        }
    }

    #[test]
    fn registry_sync_defaults_to_public_source_when_override_omitted() {
        assert_eq!(
            registry_sync_default_or_override(None),
            DEFAULT_PUBLIC_REGISTRY_SOURCE
        );
        assert_eq!(
            registry_sync_default_or_override(Some("acme-corp/internal-registry".to_string())),
            "acme-corp/internal-registry"
        );
    }

    #[test]
    fn parse_registry_list_and_search_accept_local_discovery_filters() {
        let list = vec![
            "traverse-cli".to_string(),
            "registry".to_string(),
            "list".to_string(),
            "--workspace".to_string(),
            "local-default".to_string(),
            "--namespace".to_string(),
            "traverse-starter".to_string(),
            "--id-prefix".to_string(),
            "traverse-starter.pro".to_string(),
            "--json".to_string(),
        ];
        assert!(matches!(
            parse_command(&list),
            Ok(Command::RegistryList {
                json_output: true,
                ..
            })
        ));

        let search = vec![
            "traverse-cli".to_string(),
            "registry".to_string(),
            "search".to_string(),
            "process".to_string(),
            "--workspace".to_string(),
            "local-default".to_string(),
        ];
        assert!(matches!(
            parse_command(&search),
            Ok(Command::RegistrySearch { query, .. }) if query == "process"
        ));
    }

    #[test]
    fn registry_records_sort_by_namespace_id_then_descending_semver() {
        let mut records = [
            registry_record_fixture("zeta", "same", "1.0.0"),
            registry_record_fixture("alpha", "same", "1.0.0"),
            registry_record_fixture("alpha", "same", "2.0.0"),
            registry_record_fixture("alpha", "another", "1.0.0"),
        ];
        records.sort_by(registry_record_order);
        let positions = records
            .iter()
            .map(|record| format!("{}:{}@{}", record.namespace, record.id, record.version))
            .collect::<Vec<_>>();
        assert_eq!(
            positions,
            vec![
                "alpha:another@1.0.0",
                "alpha:same@2.0.0",
                "alpha:same@1.0.0",
                "zeta:same@1.0.0",
            ]
        );
    }

    #[test]
    fn parse_capability_publish_requires_contract_artifact_registry_and_json_flags() {
        let args = vec![
            "traverse-cli".to_string(),
            "capability".to_string(),
            "publish".to_string(),
            "--contract".to_string(),
            "contracts/examples/traverse-starter/capabilities/process/contract.json".to_string(),
            "--artifact".to_string(),
            "target/traverse-starter.wasm".to_string(),
            "--registry-repo".to_string(),
            "../registry".to_string(),
            "--json".to_string(),
            "--dry-run".to_string(),
        ];

        let command = parse_command(&args).expect("capability publish should parse");

        match command {
            Command::CapabilityPublish {
                contract_path,
                artifact_path,
                registry_repo_path,
                registry_repo_remote,
                json_output,
                dry_run,
            } => {
                assert_eq!(
                    contract_path,
                    PathBuf::from(
                        "contracts/examples/traverse-starter/capabilities/process/contract.json"
                    )
                );
                assert_eq!(artifact_path, PathBuf::from("target/traverse-starter.wasm"));
                assert_eq!(registry_repo_path, PathBuf::from("../registry"));
                assert_eq!(registry_repo_remote, None);
                assert!(json_output);
                assert!(dry_run);
            }
            other => assert!(matches!(other, Command::CapabilityPublish { .. })),
        }

        let mut missing_json = args.clone();
        missing_json.retain(|arg| arg != "--json");
        assert!(parse_command(&missing_json).is_err());
    }

    #[test]
    fn parse_capability_publish_accepts_registry_repo_remote_override() {
        let args = vec![
            "traverse-cli".to_string(),
            "capability".to_string(),
            "publish".to_string(),
            "--contract".to_string(),
            "contracts/examples/traverse-starter/capabilities/process/contract.json".to_string(),
            "--artifact".to_string(),
            "target/traverse-starter.wasm".to_string(),
            "--registry-repo".to_string(),
            "../registry".to_string(),
            "--registry-repo-remote".to_string(),
            "acme-corp/internal-registry".to_string(),
            "--json".to_string(),
        ];

        let command = parse_command(&args).expect("capability publish should parse");

        match command {
            Command::CapabilityPublish {
                registry_repo_remote,
                ..
            } => {
                assert_eq!(
                    registry_repo_remote.as_deref(),
                    Some("acme-corp/internal-registry")
                );
            }
            other => assert!(matches!(other, Command::CapabilityPublish { .. })),
        }
    }

    #[test]
    fn capability_publish_dry_run_reports_plan_without_writes() {
        let fixture = capability_publish_fixture();
        let runner = RecordingPublishRunner::default();

        let output = capability_publish_at(&fixture.request(true), &runner)
            .expect("dry-run publish should return JSON");
        let json: Value = serde_json::from_str(&output).expect("publish output must be JSON");

        assert_eq!(json["status"], "dry_run");
        assert_eq!(json["registry_repo"], "traverse-framework/registry");
        assert_eq!(
            json["registry_path"],
            "capabilities/traverse-starter/traverse-starter.process/1.0.0/contract.json"
        );
        assert!(
            json["artifact_digest"]
                .as_str()
                .unwrap_or_default()
                .starts_with("sha256:")
        );
        assert_eq!(
            json["artifact_release_tag"],
            "artifacts/traverse-starter.process-1.0.0"
        );
        assert_eq!(
            json["artifact_url"],
            "https://github.com/traverse-framework/registry/releases/download/artifacts/traverse-starter.process-1.0.0/artifact.wasm"
        );
        assert!(!fixture.registry_contract_path().exists());
        assert!(runner.commands.borrow().is_empty());
    }

    #[test]
    fn capability_publish_refuses_existing_registry_path_before_commands() {
        let fixture = capability_publish_fixture();
        let target_path = fixture.registry_contract_path();
        fs::create_dir_all(target_path.parent().expect("target path must have parent"))
            .expect("target parent should create");
        fs::write(&target_path, "{}").expect("existing registry contract should write");
        let runner = RecordingPublishRunner::default();

        let output = capability_publish_at(&fixture.request(false), &runner)
            .expect("immutable conflict should return JSON");
        let json: Value = serde_json::from_str(&output).expect("publish output must be JSON");

        assert_eq!(json["status"], "failed");
        assert_eq!(
            json["errors"][0]["code"],
            "capability_publish_immutable_conflict"
        );
        assert!(runner.commands.borrow().is_empty());
    }

    #[test]
    fn capability_publish_success_prepares_registry_file_and_pr() {
        let fixture = capability_publish_fixture();
        let runner = RecordingPublishRunner::default();

        let output = capability_publish_at(&fixture.request(false), &runner)
            .expect("publish should open PR with fake runner");
        let json: Value = serde_json::from_str(&output).expect("publish output must be JSON");
        let commands = runner.commands.borrow().join("\n");

        assert_eq!(json["status"], "pr_opened");
        assert_eq!(
            json["pull_request_url"],
            "https://github.com/traverse-framework/registry/pull/123"
        );
        assert!(fixture.registry_contract_path().exists());
        let contract: Value = serde_json::from_str(
            &fs::read_to_string(fixture.registry_contract_path())
                .expect("published registry contract should read"),
        )
        .expect("published registry contract should parse");
        assert_eq!(contract["artifact"]["digest"], json["artifact_digest"]);
        assert_eq!(contract["artifact"]["url"], json["artifact_url"]);
        assert_eq!(
            contract["use_cases"].as_array().map_or(0, Vec::len),
            1,
            "publish must preserve author use_cases (spec 102 FR-005)"
        );
        assert!(
            contract["use_cases"][0]["input_example"]["note"]
                .as_str()
                .is_some(),
            "preserved use_cases must keep input_example"
        );
        assert!(
            contract["evidence"]
                .as_array()
                .is_some_and(|items| !items.is_empty()),
            "publish must preserve author evidence (spec 102 FR-005)"
        );
        assert!(commands.contains("gh release create artifacts/traverse-starter.process-1.0.0"));
        assert!(commands.contains("git checkout -B publish/traverse-starter.process-1.0.0"));
        assert!(commands.contains("gh pr create"));
        assert!(commands.contains("001-registry-foundation"));
        assert!(commands.contains("007-artifact-hosting"));
        assert!(!commands.contains("056-capability-publish"));
        assert_eq!(json["registry_repo"], "traverse-framework/registry");
        assert!(commands.contains("--repo traverse-framework/registry"));
    }

    #[test]
    fn capability_publish_registry_repo_remote_override_targets_pr_at_override() {
        let fixture = capability_publish_fixture();
        let runner = RecordingPublishRunner::default();
        let mut request = fixture.request(false);
        request.registry_repo_remote = Some("acme-corp/internal-registry".to_string());

        let output = capability_publish_at(&request, &runner)
            .expect("publish with an override should still open a PR with the fake runner");
        let json: Value = serde_json::from_str(&output).expect("publish output must be JSON");
        let commands = runner.commands.borrow().join("\n");

        assert_eq!(json["status"], "pr_opened");
        assert_eq!(json["registry_repo"], "acme-corp/internal-registry");
        assert!(commands.contains("--repo acme-corp/internal-registry"));
        assert!(!commands.contains("--repo traverse-framework/registry"));
    }

    #[test]
    fn capability_publish_pr_failure_reports_partial_state() {
        let fixture = capability_publish_fixture();
        let runner = RecordingPublishRunner {
            fail_program: None,
            fail_command_prefix: Some("gh pr create"),
            commands: RefCell::new(Vec::new()),
        };

        let output = capability_publish_at(&fixture.request(false), &runner)
            .expect("PR failure should return JSON evidence");
        let json: Value = serde_json::from_str(&output).expect("publish output must be JSON");

        assert_eq!(json["status"], "failed");
        assert_eq!(
            json["errors"][0]["code"],
            "capability_publish_pr_create_failed"
        );
        assert!(
            json["partial_state"]
                .as_str()
                .unwrap_or_default()
                .contains("publish/traverse-starter.process-1.0.0")
        );
        assert!(fixture.registry_contract_path().exists());
    }

    #[test]
    fn capability_publish_validation_failure_runs_no_commands() {
        let fixture = capability_publish_fixture();
        let mut contract: Value = serde_json::from_str(
            &fs::read_to_string(&fixture.contract).expect("contract fixture should read"),
        )
        .expect("contract fixture should parse");
        contract["version"] = Value::String("not-semver".to_string());
        fs::write(
            &fixture.contract,
            serde_json::to_string_pretty(&contract).expect("contract fixture should serialize"),
        )
        .expect("invalid contract should write");
        let runner = RecordingPublishRunner::default();

        let output = capability_publish_at(&fixture.request(false), &runner)
            .expect("validation failure should return JSON evidence");
        let json: Value = serde_json::from_str(&output).expect("publish output must be JSON");

        assert_eq!(json["status"], "failed");
        assert_eq!(
            json["errors"][0]["code"],
            "capability_publish_contract_validation_failed"
        );
        assert!(runner.commands.borrow().is_empty());
    }

    #[test]
    fn capability_publish_surface_coverage_failure_runs_no_commands() {
        let fixture = capability_publish_fixture();
        let mut contract: Value = serde_json::from_str(
            &fs::read_to_string(&fixture.contract).expect("contract fixture should read"),
        )
        .expect("contract fixture should parse");
        contract["inputs"]["schema"]["properties"]["action"] = serde_json::json!({
            "type": "string",
            "enum": ["create", "resolve"]
        });
        contract["use_cases"] = serde_json::json!([
            {
                "scenario": "create only",
                "input_example": { "note": "hello", "action": "create" },
                "output_example": { "status": "ok" },
                "happy": true
            }
        ]);
        fs::write(
            &fixture.contract,
            serde_json::to_string_pretty(&contract).expect("contract fixture should serialize"),
        )
        .expect("uncovered-action contract should write");
        let runner = RecordingPublishRunner::default();

        let output = capability_publish_at(&fixture.request(false), &runner)
            .expect("surface coverage failure should return JSON evidence");
        let json: Value = serde_json::from_str(&output).expect("publish output must be JSON");

        assert_eq!(json["status"], "failed");
        assert_eq!(
            json["errors"][0]["code"],
            "capability_publish_surface_coverage_failed"
        );
        assert!(
            json["errors"][0]["message"]
                .as_str()
                .expect("message")
                .contains("resolve"),
            "failure should name uncovered action"
        );
        assert!(runner.commands.borrow().is_empty());
    }

    #[test]
    fn capability_publish_persona_ref_unresolved_runs_no_commands() {
        let fixture = capability_publish_fixture();
        let mut contract: Value = serde_json::from_str(
            &fs::read_to_string(&fixture.contract).expect("contract fixture should read"),
        )
        .expect("contract fixture should parse");
        contract["use_cases"] = serde_json::json!([
            {
                "scenario": "missing persona",
                "persona_ref": "missing-persona-for-publish",
                "input_example": { "note": "hello" },
                "output_example": { "status": "ok" },
                "happy": true
            }
        ]);
        fs::write(
            &fixture.contract,
            serde_json::to_string_pretty(&contract).expect("contract fixture should serialize"),
        )
        .expect("missing-persona contract should write");
        let runner = RecordingPublishRunner::default();

        let output = capability_publish_at(&fixture.request(true), &runner)
            .expect("unresolved persona_ref should return JSON evidence");
        let json: Value = serde_json::from_str(&output).expect("publish output must be JSON");

        assert_eq!(json["status"], "failed");
        assert_eq!(
            json["errors"][0]["code"],
            "capability_publish_persona_ref_unresolved"
        );
        assert!(
            json["errors"][0]["message"]
                .as_str()
                .expect("message")
                .contains("missing-persona-for-publish"),
            "failure should name missing persona id"
        );
        assert!(
            json["errors"][0]["message"]
                .as_str()
                .expect("message")
                .contains("personas/<id>/<version>/persona.json"),
            "failure should suggest persona path shape"
        );
        assert!(runner.commands.borrow().is_empty());
    }

    #[test]
    fn capability_publish_persona_ref_resolves_when_present() {
        let fixture = capability_publish_fixture();
        let persona_path = fixture
            .registry_repo
            .join("personas/platform-security-engineer/1.0.0");
        fs::create_dir_all(&persona_path).expect("persona dir should create");
        fs::write(
            persona_path.join("persona.json"),
            r#"{"id":"platform-security-engineer"}"#,
        )
        .expect("persona.json should write");
        let mut contract: Value = serde_json::from_str(
            &fs::read_to_string(&fixture.contract).expect("contract fixture should read"),
        )
        .expect("contract fixture should parse");
        contract["use_cases"] = serde_json::json!([
            {
                "scenario": "present persona",
                "persona_ref": "platform-security-engineer",
                "input_example": { "note": "hello" },
                "output_example": { "status": "ok" },
                "happy": true
            }
        ]);
        fs::write(
            &fixture.contract,
            serde_json::to_string_pretty(&contract).expect("contract fixture should serialize"),
        )
        .expect("resolved-persona contract should write");
        let runner = RecordingPublishRunner::default();

        let output = capability_publish_at(&fixture.request(true), &runner)
            .expect("resolved persona_ref dry-run should succeed");
        let json: Value = serde_json::from_str(&output).expect("publish output must be JSON");

        assert_eq!(json["status"], "dry_run");
        assert!(runner.commands.borrow().is_empty());
    }

    #[test]
    fn unresolved_persona_refs_reports_gaps_and_skips_when_absent() {
        let temp_dir = unique_temp_dir();
        let persona_path = temp_dir.join("personas/client-developer/1.0.0");
        fs::create_dir_all(&persona_path).expect("persona dir should create");
        fs::write(
            persona_path.join("persona.json"),
            r#"{"id":"client-developer"}"#,
        )
        .expect("persona.json should write");

        let covered = serde_json::json!({
            "use_cases": [
                { "persona_ref": "client-developer" },
                { "name": "no persona" }
            ]
        });
        assert_eq!(
            unresolved_persona_refs(&covered, &temp_dir).expect("covered personas"),
            Vec::<String>::new()
        );

        let gap = serde_json::json!({
            "use_cases": [
                { "persona_ref": "client-developer" },
                { "persona_ref": "missing-persona" }
            ]
        });
        assert_eq!(
            unresolved_persona_refs(&gap, &temp_dir).expect("gap personas"),
            vec!["missing-persona".to_string()]
        );

        assert_eq!(
            unresolved_persona_refs(&serde_json::json!({}), &temp_dir).expect("no use_cases"),
            Vec::<String>::new()
        );

        let bad = enforce_persona_refs_resolve(
            r#"{"use_cases":[{"persona_ref":"../escape"}]}"#,
            &temp_dir,
        )
        .expect_err("unsafe persona id must fail");
        assert_eq!(bad.0, "capability_publish_persona_ref_unresolved");
    }

    #[test]
    fn uncovered_action_enum_values_reports_gaps_and_skips_when_absent() {
        let covered = serde_json::json!({
            "inputs": {
                "schema": {
                    "properties": {
                        "action": { "type": "string", "enum": ["create", "edit"] }
                    }
                }
            },
            "use_cases": [
                { "input_example": { "action": "create" } },
                { "input_example": { "action": "edit" } }
            ]
        });
        assert!(
            uncovered_action_enum_values(&covered)
                .expect("covered contract")
                .is_empty()
        );

        let gap = serde_json::json!({
            "inputs": {
                "schema": {
                    "properties": {
                        "action": { "type": "string", "enum": ["create", "pin"] }
                    }
                }
            },
            "use_cases": [
                { "input_example": { "action": "create" } }
            ]
        });
        assert_eq!(
            uncovered_action_enum_values(&gap).expect("gap contract"),
            vec!["pin".to_string()]
        );

        let no_action = serde_json::json!({
            "inputs": { "schema": { "properties": {} } },
            "use_cases": [{ "input_example": {} }]
        });
        assert!(
            uncovered_action_enum_values(&no_action)
                .expect("no action enum")
                .is_empty()
        );

        let empty_err =
            enforce_contract_surface_coverage(r#"{"inputs":{"schema":{"properties":{}}}}"#)
                .expect_err("contracts without use_cases must fail");
        assert_eq!(empty_err.0, "capability_publish_surface_coverage_failed");
        assert!(empty_err.1.contains("use_cases"));

        let err = enforce_contract_surface_coverage(
            r#"{"inputs":{"schema":{"properties":{"action":{"enum":["a","b"]}}}},"use_cases":[{"input_example":{"action":"a"}}]}"#,
        )
        .expect_err("uncovered enum must fail");
        assert_eq!(err.0, "capability_publish_surface_coverage_failed");
        assert!(err.1.contains('b'));
    }

    #[test]
    fn surface_coverage_pass_covers_required_nested_enums_and_outputs() {
        let pass = serde_json::json!({
            "inputs": {
                "schema": {
                    "required": ["note", "mode"],
                    "properties": {
                        "note": { "type": "string" },
                        "mode": { "type": "string", "enum": ["fast", "careful"] },
                        "config": {
                            "type": "object",
                            "properties": {
                                "tone": { "type": "string", "enum": ["soft", "direct"] }
                            }
                        },
                        "extra": {
                            "additionalProperties": {
                                "type": "string",
                                "enum": ["ignored-by-walker"]
                            }
                        }
                    }
                }
            },
            "outputs": {
                "schema": {
                    "properties": {
                        "reason_code": {
                            "type": "string",
                            "enum": ["ok", "bad_input"]
                        },
                        "status": {
                            "type": "string",
                            "enum": ["allow", "deny"]
                        }
                    }
                }
            },
            "use_cases": [
                {
                    "input_example": {
                        "note": "n1",
                        "mode": "fast",
                        "config": { "tone": "soft" }
                    },
                    "output_example": { "reason_code": "ok", "status": "allow" }
                },
                {
                    "input_example": {
                        "note": "n2",
                        "mode": "careful",
                        "config": { "tone": "direct" }
                    },
                    "output_example": { "reason_code": "bad_input", "status": "deny" }
                }
            ]
        });
        assert!(
            surface_coverage_gap_messages(&pass)
                .expect("pass fixture")
                .is_empty()
        );
        enforce_contract_surface_coverage(&pass.to_string()).expect("full coverage must pass");
    }

    #[test]
    fn surface_coverage_reports_missing_required_input_properties() {
        let required_gap = serde_json::json!({
            "inputs": {
                "schema": {
                    "required": ["note", "mode"],
                    "properties": {
                        "note": { "type": "string" },
                        "mode": { "type": "string", "enum": ["fast"] }
                    }
                }
            },
            "use_cases": [
                { "input_example": { "mode": "fast" }, "output_example": {} }
            ]
        });
        let required_msgs =
            surface_coverage_gap_messages(&required_gap).expect("required gap fixture");
        assert!(
            required_msgs.iter().any(|msg| msg.contains("note")),
            "expected missing required note, got {required_msgs:?}"
        );
    }

    #[test]
    fn surface_coverage_reports_nested_input_enum_gaps() {
        let nested_gap = serde_json::json!({
            "inputs": {
                "schema": {
                    "properties": {
                        "config": {
                            "properties": {
                                "tone": { "enum": ["soft", "direct"] }
                            }
                        }
                    }
                }
            },
            "use_cases": [
                { "input_example": { "config": { "tone": "soft" } }, "output_example": {} }
            ]
        });
        let nested_msgs = surface_coverage_gap_messages(&nested_gap).expect("nested gap fixture");
        assert!(
            nested_msgs
                .iter()
                .any(|msg| msg.contains("config.tone=direct")),
            "expected nested enum gap, got {nested_msgs:?}"
        );
    }

    #[test]
    fn surface_coverage_reports_output_reason_code_enum_gaps() {
        let output_gap = serde_json::json!({
            "inputs": { "schema": { "properties": {} } },
            "outputs": {
                "schema": {
                    "properties": {
                        "reason_code": { "enum": ["ok", "bad_input"] }
                    }
                }
            },
            "use_cases": [
                { "input_example": {}, "output_example": { "reason_code": "ok" } }
            ]
        });
        let output_msgs = surface_coverage_gap_messages(&output_gap).expect("output gap fixture");
        assert!(
            output_msgs
                .iter()
                .any(|msg| msg.contains("reason_code=bad_input")),
            "expected reason_code gap, got {output_msgs:?}"
        );
    }

    #[test]
    fn use_case_smoke_coverage_gaps_require_ucnn_fixtures() {
        assert!(
            use_case_smoke_coverage_gaps(
                2,
                &[
                    "uc01-happy.json".to_string(),
                    "uc02-sad.json".to_string(),
                    "extra.json".to_string()
                ]
            )
            .is_empty()
        );
        assert_eq!(
            use_case_smoke_coverage_gaps(2, &["uc01-happy.json".to_string()]),
            vec!["use_cases[1] lacks runtime-requests/uc02-*.json (spec 102 FR-007)".to_string()]
        );

        let temp_dir = unique_temp_dir();
        let package_dir = temp_dir.join("pkg");
        fs::create_dir_all(package_dir.join("runtime-requests"))
            .expect("package dirs should create");
        fs::write(
            package_dir.join("contract.json"),
            r#"{"use_cases":[{"scenario":"a"},{"scenario":"b"}]}"#,
        )
        .expect("contract should write");
        fs::write(package_dir.join("runtime-requests/uc01-a.json"), "{}")
            .expect("uc01 should write");
        let gaps = use_case_smoke_coverage_gaps_for_package(&package_dir)
            .expect("package gaps should compute");
        assert_eq!(gaps.len(), 1);
        assert!(gaps[0].contains("uc02-"));
    }

    #[test]
    fn app_validate_returns_validated_json_for_checked_in_app_manifest() {
        let manifest_path =
            repo_root().join("examples/applications/expedition-readiness/app.manifest.json");

        let output =
            app_validate(&manifest_path, None, true).expect("app validation should succeed");
        let json: Value = serde_json::from_str(&output).expect("validation output must be JSON");

        assert_eq!(json["status"], "validated");
        assert_eq!(json["app_id"], "expedition.readiness");
        assert_eq!(
            json["component_ids"][0],
            "expedition.readiness.validate-team-readiness-component"
        );
        assert_eq!(json["digest_verification"][0]["status"], "verified");
        assert_eq!(json["model_readiness"][0]["status"], "declared");
        assert_eq!(
            json["effective_config"]["redacted_secret_keys"]
                .as_array()
                .expect("redacted secret keys must be an array")
                .len(),
            0
        );
    }

    #[test]
    fn app_validate_rejects_placeholder_digest_with_failed_json() {
        let temp_dir = unique_temp_dir();
        let manifest_path = write_app_validate_fixture(
            &temp_dir,
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            None,
        );

        let output =
            app_validate(&manifest_path, None, true).expect("validation failure is JSON evidence");
        let json: Value = serde_json::from_str(&output).expect("failure output must be JSON");

        assert_eq!(json["status"], "failed");
        assert_eq!(json["errors"][0]["code"], "placeholder_wasm_digest");
        assert_eq!(json["errors"][0]["severity"], "error");
    }

    #[test]
    fn app_validate_rejects_invalid_state_machine_transition() {
        let temp_dir = unique_temp_dir();
        let manifest_path = write_app_validate_fixture(
            &temp_dir,
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            None,
        );
        let mut manifest: Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest must read"))
                .expect("manifest must parse");
        manifest["state_machine"]["states"][0]["transitions"][0]["to"] =
            Value::String("missing".to_string());
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).expect("manifest must serialize"),
        )
        .expect("manifest must write");

        let output =
            app_validate(&manifest_path, None, true).expect("validation failure is JSON evidence");
        let json: Value = serde_json::from_str(&output).expect("failure output must be JSON");

        assert_eq!(json["status"], "failed");
        assert_eq!(
            json["errors"][0]["code"],
            "app_state_machine_undefined_state"
        );
    }

    #[test]
    fn app_validate_rejects_state_machine_unknown_capability() {
        let temp_dir = unique_temp_dir();
        let manifest_path = write_app_validate_fixture(
            &temp_dir,
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            None,
        );
        let mut manifest: Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path).expect("manifest must read"))
                .expect("manifest must parse");
        manifest["state_machine"]["states"][1]["invoke"]["capability_id"] =
            Value::String("unknown.capability".to_string());
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).expect("manifest must serialize"),
        )
        .expect("manifest must write");

        let output =
            app_validate(&manifest_path, None, true).expect("validation failure is JSON evidence");
        let json: Value = serde_json::from_str(&output).expect("failure output must be JSON");

        assert_eq!(json["status"], "failed");
        let codes = json["errors"]
            .as_array()
            .expect("errors must be an array")
            .iter()
            .map(|error| error["code"].as_str().unwrap_or_default())
            .collect::<Vec<_>>();
        assert!(codes.contains(&"app_state_machine_undefined_capability"));
    }

    #[test]
    fn app_validate_redacts_workspace_secret_keys() {
        let temp_dir = unique_temp_dir();
        let manifest_path = write_app_validate_fixture(
            &temp_dir,
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            Some(serde_json::json!({
                "overrides": {
                    "readiness_mode": "deterministic"
                },
                "secrets": {
                    "ollama_api_key": "do-not-render"
                }
            })),
        );

        let output =
            app_validate(&manifest_path, None, true).expect("app validation should succeed");
        let json: Value = serde_json::from_str(&output).expect("validation output must be JSON");

        assert_eq!(json["status"], "validated");
        assert_eq!(json["state_machine"]["initial_state"], "idle");
        assert_eq!(
            json["effective_config"]["redacted_secret_keys"][0],
            "ollama_api_key"
        );
        assert!(!output.contains("do-not-render"));
    }

    #[test]
    fn app_register_persists_durable_workspace_state() {
        let state_root = unique_temp_dir();
        let manifest_path =
            repo_root().join("examples/applications/expedition-readiness/app.manifest.json");

        let output = app_register_at(&state_root, &manifest_path, "local", true)
            .expect("app registration should succeed");
        let json: Value = serde_json::from_str(&output).expect("registration output must be JSON");
        let state_path =
            app_registration_state_path(&state_root, "local", "expedition.readiness", "1.0.0");
        let persisted: Value = serde_json::from_str(
            &fs::read_to_string(&state_path).expect("registration state must persist"),
        )
        .expect("persisted state must be JSON");

        assert_eq!(json["status"], "registered");
        assert_eq!(json["workspace_id"], "local");
        assert_eq!(json["app_id"], "expedition.readiness");
        assert_eq!(json["state_scope"], "workspace_persisted");
        assert_eq!(
            json["component_ids"][0],
            "expedition.readiness.validate-team-readiness-component"
        );
        assert_eq!(json, persisted);
    }

    #[test]
    fn app_register_is_idempotent_for_unchanged_bundle() {
        let state_root = unique_temp_dir();
        let manifest_path =
            repo_root().join("examples/applications/expedition-readiness/app.manifest.json");

        let first = app_register_at(&state_root, &manifest_path, "local", true)
            .expect("first registration should succeed");
        let second = app_register_at(&state_root, &manifest_path, "local", true)
            .expect("second registration should succeed");
        let first_json: Value =
            serde_json::from_str(&first).expect("first registration output must be JSON");
        let second_json: Value =
            serde_json::from_str(&second).expect("second registration output must be JSON");

        assert_eq!(first_json["status"], "registered");
        assert_eq!(second_json["status"], "already_registered");
        assert_eq!(
            first_json["registration_fingerprint"],
            second_json["registration_fingerprint"]
        );
    }

    #[test]
    fn app_activate_persists_empty_connector_activation_evidence() {
        let state_root = unique_temp_dir();
        let fixture_root = unique_temp_dir();
        let manifest_path = write_app_validate_fixture(
            &fixture_root,
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            None,
        );
        let host_activation_path = fixture_root.join("host-activation.json");
        fs::write(
            &host_activation_path,
            r#"{"connectors":[],"artifacts":[{"contract_reference":"expedition.planning.validate-team-readiness@1.0.0","placement_target":"local","candidates":[{"package_id":"fixture.team-readiness","package_version":"1.0.0","digest":"sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90","abi":"wasi-preview1","lifecycle":"active","placement":["local"],"execution_constraints":"fixture"}]}]}"#,
        )
            .expect("host activation fixture should write");

        let output = app_activate_at(
            &state_root,
            &manifest_path,
            "local",
            &host_activation_path,
            true,
        )
        .expect("empty connector activation should succeed");
        let json: Value = serde_json::from_str(&output).expect("activation output must be JSON");
        let state_path =
            app_activation_state_path(&state_root, "local", "expedition.readiness", "1.0.0");
        let persisted: Value = serde_json::from_str(
            &fs::read_to_string(&state_path).expect("activation evidence must persist"),
        )
        .expect("persisted activation must be JSON");

        assert_eq!(json["status"], "activated");
        assert_eq!(json["connectors"], serde_json::json!([]));
        assert_eq!(
            json["artifacts"][0]["selected_package_id"],
            "fixture.team-readiness"
        );
        assert_eq!(json, persisted);
    }

    #[test]
    fn app_activate_records_universal_connector_fixture_evidence_without_config_values() {
        let state_root = unique_temp_dir();
        let fixture_root = unique_temp_dir();
        let manifest_path = write_app_validate_fixture(
            &fixture_root,
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            None,
        );
        let mut manifest: Value = serde_json::from_str(
            &fs::read_to_string(&manifest_path).expect("fixture manifest should read"),
        )
        .expect("fixture manifest should parse");
        manifest["connector_bindings"] = serde_json::json!([
            {"connector_id":"traverse.object-store","version_range":"^1.0.0","config_ref":"object-store-authority"},
            {"connector_id":"traverse.state-store","version_range":"^1.0.0","config_ref":"state-store-authority"},
            {"connector_id":"traverse.scheduler","version_range":"^1.0.0","config_ref":"scheduler-authority"}
        ]);
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).expect("fixture manifest should serialize"),
        )
        .expect("fixture manifest should write");
        let host_activation_path = fixture_root.join("host-activation.json");
        fs::write(
            &host_activation_path,
            r#"{"connectors":[{"connector_id":"traverse.object-store","installed_version":"1.0.0","placement_target":"local","config":{"authority_ref":"vault:object-store"}},{"connector_id":"traverse.state-store","installed_version":"1.0.0","placement_target":"local","config":{"authority_ref":"vault:state-store"}},{"connector_id":"traverse.scheduler","installed_version":"1.0.0","placement_target":"local","config":{"authority_ref":"vault:scheduler"}}],"artifacts":[{"contract_reference":"expedition.planning.validate-team-readiness@1.0.0","placement_target":"local","candidates":[{"package_id":"fixture.team-readiness","package_version":"1.0.0","digest":"sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90","abi":"wasi-preview1","lifecycle":"active","placement":["local"],"execution_constraints":"fixture"}]}]}"#,
        )
        .expect("host activation fixture should write");

        let output = app_activate_at(
            &state_root,
            &manifest_path,
            "local",
            &host_activation_path,
            true,
        )
        .expect("universal connector activation should succeed");
        let json: Value = serde_json::from_str(&output).expect("activation output must be JSON");

        assert_eq!(json["status"], "activated");
        assert_eq!(json["connectors"].as_array().map(Vec::len), Some(3));
        assert_eq!(
            json["connectors"][0]["connector_id"],
            "traverse.object-store"
        );
        assert_eq!(json["connectors"][1]["connector_id"], "traverse.scheduler");
        assert_eq!(
            json["connectors"][2]["connector_id"],
            "traverse.state-store"
        );
        for private_marker in ["vault:", "secret", "endpoint", "bucket"] {
            assert!(
                !output.contains(private_marker),
                "activation evidence leaked {private_marker}"
            );
        }
    }

    #[test]
    fn app_activate_fails_closed_when_required_artifact_input_is_missing() {
        let state_root = unique_temp_dir();
        let fixture_root = unique_temp_dir();
        let manifest_path = write_app_validate_fixture(
            &fixture_root,
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            None,
        );
        let host_activation_path = fixture_root.join("host-activation.json");
        fs::write(&host_activation_path, r#"{"connectors":[]}"#)
            .expect("host activation fixture should write");

        let output = app_activate_at(
            &state_root,
            &manifest_path,
            "local",
            &host_activation_path,
            true,
        )
        .expect("activation denial should render JSON evidence");
        let json: Value = serde_json::from_str(&output).expect("activation output must be JSON");

        assert_eq!(json["status"], "activation_failed");
        assert_eq!(json["errors"][0]["code"], "executable_artifact_unavailable");
        assert!(!state_root.join(".traverse").exists());
    }

    #[test]
    fn app_activate_honors_an_exact_executable_package_pin() {
        let state_root = unique_temp_dir();
        let fixture_root = unique_temp_dir();
        let manifest_path = write_app_validate_fixture(
            &fixture_root,
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            None,
        );
        let app_manifest: Value = serde_json::from_str(
            &fs::read_to_string(&manifest_path).expect("app manifest should read"),
        )
        .expect("app manifest should parse");
        let component_path = PathBuf::from(
            app_manifest["components"][0]["manifest_path"]
                .as_str()
                .expect("component manifest path should be present"),
        );
        let mut component: Value = serde_json::from_str(
            &fs::read_to_string(&component_path).expect("component manifest should read"),
        )
        .expect("component manifest should parse");
        component["executable_pin"] = serde_json::json!({
            "package_id": "fixture.pinned",
            "package_version": "1.0.0"
        });
        fs::write(
            &component_path,
            serde_json::to_string_pretty(&component).expect("component manifest should serialize"),
        )
        .expect("component manifest should write");
        let host_activation_path = fixture_root.join("host-activation.json");
        fs::write(
            &host_activation_path,
            r#"{"connectors":[],"artifacts":[{"contract_reference":"expedition.planning.validate-team-readiness@1.0.0","placement_target":"local","candidates":[{"package_id":"fixture.newer","package_version":"2.0.0","digest":"sha256:newer","abi":"wasi-preview1","lifecycle":"active","placement":["local"],"execution_constraints":"fixture"},{"package_id":"fixture.pinned","package_version":"1.0.0","digest":"sha256:pinned","abi":"wasi-preview1","lifecycle":"active","placement":["local"],"execution_constraints":"fixture"}]}]}"#,
        )
        .expect("host activation fixture should write");

        let output = app_activate_at(
            &state_root,
            &manifest_path,
            "local",
            &host_activation_path,
            true,
        )
        .expect("pinned activation should succeed");
        let json: Value = serde_json::from_str(&output).expect("activation output must be JSON");

        assert_eq!(json["status"], "activated");
        assert_eq!(
            json["artifacts"][0]["selected_package_id"],
            "fixture.pinned"
        );
        assert_eq!(json["artifacts"][0]["selected_digest"], "sha256:pinned");
    }

    #[test]
    fn app_activate_rejects_undeclared_host_connector_without_writing_state() {
        let state_root = unique_temp_dir();
        let fixture_root = unique_temp_dir();
        let manifest_path = write_app_validate_fixture(
            &fixture_root,
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            None,
        );
        let host_activation_path = fixture_root.join("host-activation.json");
        fs::write(
            &host_activation_path,
            r#"{"connectors":[{"connector_id":"unknown.connector","installed_version":"1.0.0","placement_target":"local","config":{}}]}"#,
        )
        .expect("host activation fixture should write");

        let output = app_activate_at(
            &state_root,
            &manifest_path,
            "local",
            &host_activation_path,
            true,
        )
        .expect("activation denial should render JSON evidence");
        let json: Value = serde_json::from_str(&output).expect("activation output must be JSON");

        assert_eq!(json["status"], "activation_failed");
        assert_eq!(json["errors"][0]["code"], "connector_activation_undeclared");
        assert!(!state_root.join(".traverse").exists());
    }

    #[test]
    fn app_activate_persists_universal_connector_evidence_without_private_config_values() {
        let state_root = unique_temp_dir();
        let fixture_root = unique_temp_dir();
        let manifest_path = write_app_validate_fixture(
            &fixture_root,
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            None,
        );
        add_universal_connector_bindings(&manifest_path);
        let host_activation_path = fixture_root.join("host-activation.json");
        fs::write(
            &host_activation_path,
            universal_host_activation_json(&serde_json::json!([
                {
                    "connector_id": "traverse.object-store",
                    "installed_version": "1.0.0",
                    "placement_target": "local",
                    "config": {
                        "authority_ref": "bucket-prod-private-name",
                        "retention_classes": ["standard"]
                    }
                },
                {
                    "connector_id": "traverse.state-store",
                    "installed_version": "1.0.0",
                    "placement_target": "local",
                    "config": {
                        "authority_ref": "postgres://private-state-store",
                        "record_type_namespace": "fixture"
                    }
                },
                {
                    "connector_id": "traverse.scheduler",
                    "installed_version": "1.0.0",
                    "placement_target": "local",
                    "config": {
                        "authority_ref": "scheduler-device-42",
                        "allowed_job_kinds": ["fixture-job"]
                    }
                }
            ])),
        )
        .expect("host activation fixture should write");

        let output = app_activate_at(
            &state_root,
            &manifest_path,
            "local",
            &host_activation_path,
            true,
        )
        .expect("universal connector activation should succeed");
        let json: Value = serde_json::from_str(&output).expect("activation output must be JSON");
        let state_path =
            app_activation_state_path(&state_root, "local", "expedition.readiness", "1.0.0");
        let persisted: Value = serde_json::from_str(
            &fs::read_to_string(&state_path).expect("activation evidence must persist"),
        )
        .expect("persisted activation must be JSON");

        assert_eq!(json["status"], "activated");
        assert_eq!(json["connectors"].as_array().map(Vec::len), Some(3));
        assert_eq!(
            json["connectors"][0]["connector_id"],
            "traverse.object-store"
        );
        assert_eq!(json["connectors"][1]["connector_id"], "traverse.scheduler");
        assert_eq!(
            json["connectors"][2]["connector_id"],
            "traverse.state-store"
        );
        assert_eq!(
            json["connectors"][0]["config_keys_present"],
            serde_json::json!(["authority_ref", "retention_classes"])
        );
        assert_eq!(json, persisted);
        for private_value in [
            "bucket-prod-private-name",
            "postgres://private-state-store",
            "scheduler-device-42",
        ] {
            assert!(
                !output.contains(private_value),
                "activation evidence leaked host-private value {private_value}"
            );
        }
    }

    #[test]
    fn app_activate_rejects_unconfigured_universal_connector_without_leaking_value() {
        let state_root = unique_temp_dir();
        let fixture_root = unique_temp_dir();
        let manifest_path = write_app_validate_fixture(
            &fixture_root,
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            None,
        );
        add_universal_connector_bindings(&manifest_path);
        let host_activation_path = fixture_root.join("host-activation.json");
        fs::write(
            &host_activation_path,
            universal_host_activation_json(&serde_json::json!([
                {
                    "connector_id": "traverse.object-store",
                    "installed_version": "1.0.0",
                    "placement_target": "local",
                    "config": {
                        "retention_classes": ["standard"],
                        "private_value": "bucket-prod-private-name"
                    }
                },
                {
                    "connector_id": "traverse.state-store",
                    "installed_version": "1.0.0",
                    "placement_target": "local",
                    "config": {
                        "authority_ref": "state-authority",
                        "record_type_namespace": "fixture"
                    }
                },
                {
                    "connector_id": "traverse.scheduler",
                    "installed_version": "1.0.0",
                    "placement_target": "local",
                    "config": {
                        "authority_ref": "scheduler-authority",
                        "allowed_job_kinds": ["fixture-job"]
                    }
                }
            ])),
        )
        .expect("host activation fixture should write");

        let output = app_activate_at(
            &state_root,
            &manifest_path,
            "local",
            &host_activation_path,
            true,
        )
        .expect("activation denial should render JSON evidence");
        let json: Value = serde_json::from_str(&output).expect("activation output must be JSON");

        assert_eq!(json["status"], "activation_failed");
        assert_eq!(json["errors"][0]["code"], "connector_unconfigured");
        assert_eq!(
            json["errors"][0]["path"],
            "$.connector_bindings[traverse.object-store]"
        );
        assert!(
            !output.contains("bucket-prod-private-name"),
            "activation failure leaked host-private value"
        );
        assert!(!state_root.join(".traverse").exists());
    }

    #[test]
    #[allow(clippy::too_many_lines)] // Exercises the complete synced-reference registration flow.
    fn synced_registry_reference_validates_registers_and_reuses_verified_cache() {
        let state_root = unique_temp_dir();
        let fixture_root = unique_temp_dir();
        let repo = repo_root();
        let contract_path = repo.join(
            "contracts/examples/expedition/capabilities/validate-team-readiness/contract.json",
        );
        let artifact_path = repo.join(
            "examples/capabilities/team-readiness-agent/artifacts/validate-team-readiness-agent.wasm",
        );
        let contract_digest = format!(
            "sha256:{}",
            sha256_hex(&fs::read(&contract_path).expect("contract artifact should read"))
        );
        let artifact_digest = format!(
            "sha256:{}",
            sha256_hex(&fs::read(&artifact_path).expect("wasm artifact should read"))
        );
        let manifest_path =
            write_registry_ref_app_fixture(&fixture_root, &artifact_digest, "^1.0.0");
        write_synced_public_registry_state(
            &state_root,
            "local",
            "fixture-registry",
            "fixture-v1",
            "2026-07-22T00:00:00Z",
            PublicRegistryIndex {
                index_version: 1,
                generated_at: "2026-07-22T00:00:00Z".to_string(),
                source_commit: None,
                capabilities: vec![PublicRegistryCapabilityRecord {
                    namespace: "fixture".to_string(),
                    id: "expedition.planning.validate-team-readiness".to_string(),
                    version: "1.0.0".to_string(),
                    digest: artifact_digest.clone(),
                    artifact_url: format!("file://{}", artifact_path.display()),
                    contract_digest: contract_digest.clone(),
                    contract_url: format!("file://{}", contract_path.display()),
                    deprecated: false,
                }],
            },
        )
        .expect("synced fixture state should persist");

        let validation = app_validate_at(&state_root, &manifest_path, Some("local"), true)
            .expect("synced registry component should validate");
        let validation_json: Value =
            serde_json::from_str(&validation).expect("validation output must be JSON");
        assert_eq!(validation_json["status"], "validated");

        let registration = app_register_at(&state_root, &manifest_path, "local", true)
            .expect("synced registry component should register");
        let registration_json: Value =
            serde_json::from_str(&registration).expect("registration output must be JSON");
        let cache_root = state_root.join(".traverse/cache/sha256");
        assert_eq!(registration_json["status"], "registered");
        assert_eq!(
            registration_json["components"][0]["artifact_ref"],
            Value::String(
                cache_root
                    .join(artifact_digest.trim_start_matches("sha256:"))
                    .display()
                    .to_string()
            )
        );
        assert!(
            cache_root
                .join(contract_digest.trim_start_matches("sha256:"))
                .exists()
        );
        assert!(
            cache_root
                .join(artifact_digest.trim_start_matches("sha256:"))
                .exists()
        );
    }

    #[test]
    fn registry_reference_without_sync_returns_actionable_error() {
        let state_root = unique_temp_dir();
        let fixture_root = unique_temp_dir();
        let manifest_path = write_registry_ref_app_fixture(
            &fixture_root,
            "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            "^1.0.0",
        );

        let unsynced = app_register_at(&state_root, &manifest_path, "local", true)
            .expect("missing sync should render stable JSON evidence");
        let unsynced_json: Value =
            serde_json::from_str(&unsynced).expect("registration output must be JSON");
        assert_eq!(unsynced_json["status"], "failed");
        assert_eq!(
            unsynced_json["errors"][0]["code"],
            "registry_reference_requires_resolution"
        );
        assert!(
            unsynced_json["errors"][0]["message"]
                .as_str()
                .is_some_and(|message| message
                    == "workspace local has no synced public registry state; run traverse-cli registry sync")
        );
    }

    #[test]
    fn deprecated_only_registry_range_fails_closed() {
        let state_root = unique_temp_dir();
        let fixture_root = unique_temp_dir();
        let digest = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
        let manifest_path =
            write_registry_ref_app_fixture(&fixture_root, digest, ">=1.0.0, <1.1.0");
        let mut deprecated = registry_record_fixture(
            "fixture",
            "expedition.planning.validate-team-readiness",
            "1.0.1",
        );
        deprecated.digest = digest.to_string();
        deprecated.deprecated = true;
        let active = registry_record_fixture(
            "fixture",
            "expedition.planning.validate-team-readiness",
            "1.1.0",
        );
        write_synced_public_registry_state(
            &state_root,
            "local",
            "fixture-registry",
            "fixture-v1",
            "2026-07-22T00:00:00Z",
            PublicRegistryIndex {
                index_version: 1,
                generated_at: "2026-07-22T00:00:00Z".to_string(),
                source_commit: None,
                capabilities: vec![deprecated, active],
            },
        )
        .expect("synced fixture state should persist");

        let output = app_register_at(&state_root, &manifest_path, "local", true)
            .expect("deprecated-only range should render stable JSON evidence");
        let json: Value = serde_json::from_str(&output).expect("registration output must be JSON");

        assert_eq!(json["status"], "failed");
        assert_eq!(
            json["errors"][0]["code"],
            "registry_reference_requires_resolution"
        );
        assert_eq!(
            json["errors"][0]["message"],
            "only deprecated public registry versions for fixture:expedition.planning.validate-team-readiness satisfy >=1.0.0, <1.1.0"
        );
        assert!(!state_root.join(".traverse/workspaces/local/apps").exists());
    }

    #[test]
    fn app_register_validation_failure_leaves_no_workspace_state() {
        let state_root = unique_temp_dir();
        let fixture_root = unique_temp_dir();
        let manifest_path = write_app_validate_fixture(
            &fixture_root,
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            None,
        );

        let output = app_register_at(&state_root, &manifest_path, "local", true)
            .expect("validation failure should return JSON evidence");
        let json: Value = serde_json::from_str(&output).expect("registration failure must be JSON");

        assert_eq!(json["status"], "failed");
        assert_eq!(json["errors"][0]["code"], "placeholder_wasm_digest");
        assert!(!state_root.join(".traverse").exists());
    }

    #[test]
    fn app_register_write_failure_leaves_no_partial_registration_file() {
        let state_root = unique_temp_dir();
        let manifest_path =
            repo_root().join("examples/applications/expedition-readiness/app.manifest.json");
        let apps_path = state_root.join(".traverse/workspaces/local/apps");
        fs::create_dir_all(apps_path.parent().expect("apps path must have parent"))
            .expect("workspace parent should create");
        fs::write(&apps_path, "not a directory").expect("conflicting apps path should write");

        let output = app_register_at(&state_root, &manifest_path, "local", true)
            .expect("write failure should return JSON evidence");
        let json: Value = serde_json::from_str(&output).expect("registration failure must be JSON");
        let state_path =
            app_registration_state_path(&state_root, "local", "expedition.readiness", "1.0.0");

        assert_eq!(json["status"], "failed");
        assert_eq!(json["errors"][0]["code"], "workspace_state_write_failed");
        assert!(!state_path.exists());
    }

    #[test]
    fn app_register_redacts_workspace_secret_keys() {
        let state_root = unique_temp_dir();
        let fixture_root = unique_temp_dir();
        let manifest_path = write_app_validate_fixture(
            &fixture_root,
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
            Some(serde_json::json!({
                "overrides": {
                    "readiness_mode": "deterministic"
                },
                "secrets": {
                    "ollama_api_key": "do-not-render"
                }
            })),
        );

        let output = app_register_at(&state_root, &manifest_path, "local", true)
            .expect("app registration should succeed");
        let json: Value = serde_json::from_str(&output).expect("registration output must be JSON");

        assert_eq!(json["status"], "registered");
        assert_eq!(
            json["effective_config"]["redacted_secret_keys"][0],
            "ollama_api_key"
        );
        assert!(!output.contains("do-not-render"));
    }

    #[test]
    fn registry_sync_writes_public_index_state_and_json_evidence() {
        let state_root = unique_temp_dir();
        let fetcher = StaticRegistryFetcher {
            result: Ok(FetchedRegistryIndex {
                source_repo: "traverse-framework/registry".to_string(),
                release_tag: "index-v7".to_string(),
                index: registry_index_fixture(),
            }),
        };

        let output = registry_sync_at(&state_root, "local-default", true, &fetcher)
            .expect("registry sync should succeed");
        let json: Value = serde_json::from_str(&output).expect("sync output must be JSON");
        let state = load_synced_public_registry_state(&state_root, "local-default")
            .expect("synced public registry state should load");

        assert_eq!(json["status"], "synced");
        assert_eq!(json["source"], "traverse-framework/registry");
        assert_eq!(json["release_tag"], "index-v7");
        assert_eq!(json["index_version"], 7);
        assert_eq!(json["record_count"], 1);
        assert_eq!(json["workspace"], "local-default");
        assert_eq!(state.record_count, 1);
        assert_eq!(state.capabilities[0].id, "traverse-starter.process");
    }

    #[test]
    fn registry_sync_malformed_index_leaves_existing_state_unchanged() {
        let state_root = unique_temp_dir();
        let first_fetcher = StaticRegistryFetcher {
            result: Ok(FetchedRegistryIndex {
                source_repo: "traverse-framework/registry".to_string(),
                release_tag: "index-v7".to_string(),
                index: registry_index_fixture(),
            }),
        };
        registry_sync_at(&state_root, "local-default", true, &first_fetcher)
            .expect("initial registry sync should succeed");
        let state_path =
            traverse_registry::synced_public_registry_state_path(&state_root, "local-default");
        let before = fs::read_to_string(&state_path).expect("synced state should read");

        let mut malformed = registry_index_fixture();
        malformed.capabilities[0].artifact_url = String::new();
        let bad_fetcher = StaticRegistryFetcher {
            result: Ok(FetchedRegistryIndex {
                source_repo: "traverse-framework/registry".to_string(),
                release_tag: "index-v8".to_string(),
                index: malformed,
            }),
        };
        let failure = registry_sync_at(&state_root, "local-default", true, &bad_fetcher)
            .expect_err("malformed index should fail");
        let after = fs::read_to_string(&state_path).expect("synced state should remain");

        assert!(failure.message().contains("artifact_url"));
        assert_eq!(before, after);
    }

    #[test]
    fn latest_index_release_asset_selects_highest_index_version() {
        let releases = serde_json::json!([
            {
                "tag_name": "index-v7",
                "assets": [
                    {
                        "name": "index.json",
                        "browser_download_url": "https://example.test/index-v7/index.json"
                    }
                ]
            },
            {
                "tag_name": "notes-v1",
                "assets": []
            },
            {
                "tag_name": "index-v9",
                "assets": [
                    {
                        "name": "index.json",
                        "browser_download_url": "https://example.test/index-v9/index.json"
                    }
                ]
            }
        ]);

        let (tag, url) =
            latest_index_release_asset(&releases).expect("latest release should resolve");

        assert_eq!(tag, "index-v9");
        assert_eq!(url, "https://example.test/index-v9/index.json");
    }

    #[test]
    fn app_new_generates_schema_valid_empty_bundle_structure() {
        let temp_dir = unique_temp_dir();

        let output =
            app_new_at(&temp_dir, "youaskm3", false, None).expect("app scaffold should be created");

        let app_dir = temp_dir.join("apps/youaskm3");
        assert!(output.contains("created_app: youaskm3"));
        assert!(app_dir.join("manifest.json").is_file());
        assert!(app_dir.join("workspace.config.json").is_file());
        assert!(app_dir.join("components/README.md").is_file());
        assert!(app_dir.join("workflows/README.md").is_file());
        assert!(app_dir.join("README.md").is_file());

        let manifest = load_application_bundle_manifest(&app_dir.join("manifest.json"))
            .expect("empty app manifest should be schema-valid");
        assert_eq!(manifest.app_id, "youaskm3");
        assert!(manifest.components.is_empty());
        assert!(manifest.workflows.is_empty());
        assert!(!read_tree(&app_dir).contains("TODO"));
    }

    #[test]
    fn app_new_register_rejects_incomplete_generated_bundle() {
        let temp_dir = unique_temp_dir();

        let error = app_new_at(&temp_dir, "youaskm3", true, Some("local-dev"))
            .expect_err("empty generated bundle must not register");

        assert!(
            error
                .message()
                .contains("app bundle youaskm3 is incomplete")
        );
        assert!(temp_dir.join("apps/youaskm3/manifest.json").is_file());
    }

    #[test]
    fn app_new_rejects_invalid_and_existing_scaffold_ids_without_overwriting_files() {
        let temp_dir = unique_temp_dir();

        let invalid = app_new_at(&temp_dir, "not a valid app", false, None)
            .expect_err("spaces must be rejected in scaffold ids");
        assert!(invalid.message().contains("app id"));
        assert!(!temp_dir.join("apps/not a valid app").exists());

        app_new_at(&temp_dir, "youaskm3", false, None).expect("first scaffold should be created");
        let duplicate = app_new_at(&temp_dir, "youaskm3", false, None)
            .expect_err("an existing scaffold must not be overwritten");
        assert!(
            duplicate
                .message()
                .contains("app scaffold target already exists")
        );
        assert!(temp_dir.join("apps/youaskm3/manifest.json").is_file());
    }

    #[test]
    fn component_new_redirects_to_capability_new() {
        let error = component_new("knowledge.retrieve").expect_err("component new must fail");
        assert!(matches!(error, CliError::UsageError(_)));
        assert!(error.message().contains("capability new"));
        assert!(error.message().contains("retired"));
    }

    #[test]
    fn capability_new_generates_loadable_capability_package() {
        let temp_dir = unique_temp_dir();

        let output = capability_new_at(&temp_dir, "knowledge.retrieve")
            .expect("capability scaffold should be created");

        let capability_dir = temp_dir.join("capabilities/knowledge.retrieve");
        assert!(output.contains("created_capability: knowledge.retrieve"));
        assert!(capability_dir.join("manifest.json").is_file());
        assert!(capability_dir.join("contract.json").is_file());
        assert!(capability_dir.join("src/agent.rs").is_file());
        assert!(capability_dir.join("build-fixture.sh").is_file());
        assert!(capability_dir.join("artifacts/README.md").is_file());
        let build_fixture = fs::read_to_string(capability_dir.join("build-fixture.sh"))
            .expect("build-fixture.sh should read");
        assert!(
            build_fixture.contains("expected_digest"),
            "build-fixture.sh must auto-write binary.expected_digest"
        );
        assert!(
            build_fixture.contains("0xcbf29ce484222325"),
            "build-fixture.sh must compute fnv1a64 digests"
        );
        assert!(
            capability_dir
                .join("runtime-requests/retrieve.json")
                .is_file()
        );
        assert!(!capability_dir.join("artifacts/retrieve.wasm").exists());

        let manifest = serde_json::from_str::<serde_json::Value>(
            &fs::read_to_string(capability_dir.join("manifest.json"))
                .expect("capability manifest should read"),
        )
        .expect("capability manifest should parse as JSON");
        assert_eq!(manifest["kind"], "capability_package");
        assert_eq!(manifest["package_id"], "knowledge.retrieve");
        assert_eq!(manifest["capability_ref"]["id"], "knowledge.retrieve");
        assert_eq!(manifest["capability_ref"]["contract_path"], "contract.json");
        assert_eq!(manifest["binary"]["path"], "artifacts/retrieve.wasm");
        assert_eq!(
            manifest["binary"]["abi_version"],
            SUPPORTED_HOST_ABI_VERSION
        );
        assert!(
            manifest["known_compositions"]
                .as_array()
                .is_some_and(|refs| !refs.is_empty())
        );
        assert!(manifest.get("workflow_refs").is_none());

        let contract_contents =
            fs::read_to_string(capability_dir.join("contract.json")).expect("contract should read");
        let contract = parse_contract(&contract_contents).expect("contract should parse");
        assert_eq!(contract.id, "knowledge.retrieve");
        assert_eq!(contract.lifecycle, traverse_contracts::Lifecycle::Active);
        assert!(!read_tree(&capability_dir).contains("TODO"));

        let request = serde_json::from_str::<serde_json::Value>(
            &fs::read_to_string(capability_dir.join("runtime-requests/retrieve.json"))
                .expect("sample request should read"),
        )
        .expect("sample request should parse as JSON");
        assert_eq!(request["intent"]["capability_id"], "knowledge.retrieve");
    }

    #[test]
    fn capability_new_rejects_invalid_id_without_writing_files() {
        let temp_dir = unique_temp_dir();

        let error =
            capability_new_at(&temp_dir, "../escape").expect_err("invalid id must be rejected");
        assert!(matches!(error, CliError::UsageError(_)));
        assert!(!temp_dir.join("capabilities").exists());
    }

    #[test]
    fn capability_new_refuses_to_overwrite_existing_target() {
        let temp_dir = unique_temp_dir();
        capability_new_at(&temp_dir, "knowledge.retrieve")
            .expect("first scaffold should be created");

        let error = capability_new_at(&temp_dir, "knowledge.retrieve")
            .expect_err("second scaffold must be rejected");
        assert!(matches!(error, CliError::IoError(message) if message.contains("already exists")));
    }

    #[test]
    fn parse_serve_defaults_to_loopback_8787() {
        let args = vec!["traverse-cli".to_string(), "serve".to_string()];

        let command = parse_command(&args).expect("serve command should parse");

        match command {
            Command::Serve {
                bind_address,
                auth_mode,
                allow_unauthenticated,
                allowed_origins,
                render_mobile_qr,
                ..
            } => {
                assert_eq!(bind_address, "127.0.0.1:8787");
                assert_eq!(auth_mode, None);
                assert!(!allow_unauthenticated);
                assert!(allowed_origins.is_empty());
                assert!(!render_mobile_qr);
            }
            other => assert!(matches!(other, Command::Serve { .. })),
        }
    }

    #[test]
    fn parse_serve_accepts_bind_override() {
        let args = vec![
            "traverse-cli".to_string(),
            "serve".to_string(),
            "--bind".to_string(),
            "127.0.0.1:9090".to_string(),
        ];

        let command = parse_command(&args).expect("serve command should parse");

        match command {
            Command::Serve { bind_address, .. } => {
                assert_eq!(bind_address, "127.0.0.1:9090");
            }
            other => assert!(matches!(other, Command::Serve { .. })),
        }
    }

    #[test]
    fn parse_serve_keeps_port_as_loopback_shortcut() {
        let args = vec![
            "traverse-cli".to_string(),
            "serve".to_string(),
            "--port".to_string(),
            "9090".to_string(),
            "--allow-unauthenticated".to_string(),
        ];

        let command = parse_command(&args).expect("serve command should parse");

        match command {
            Command::Serve {
                bind_address,
                auth_mode,
                allow_unauthenticated,
                render_mobile_qr,
                ..
            } => {
                assert_eq!(bind_address, "127.0.0.1:9090");
                assert_eq!(auth_mode, None);
                assert!(allow_unauthenticated);
                assert!(!render_mobile_qr);
            }
            other => assert!(matches!(other, Command::Serve { .. })),
        }
    }

    #[test]
    fn parse_serve_rejects_bind_and_port_together() {
        let args = vec![
            "traverse-cli".to_string(),
            "serve".to_string(),
            "--bind".to_string(),
            "127.0.0.1:9090".to_string(),
            "--port".to_string(),
            "9091".to_string(),
        ];

        let error = parse_command(&args).expect_err("bind plus port should be rejected");
        assert!(error.contains("--bind and --port cannot be used together"));
    }

    #[test]
    fn parse_serve_accepts_repeatable_allow_origin() {
        let args = vec![
            "traverse-cli".to_string(),
            "serve".to_string(),
            "--allow-origin".to_string(),
            "https://app.example".to_string(),
            "--allow-origin".to_string(),
            "https://admin.example".to_string(),
        ];

        let command = parse_command(&args).expect("serve command should parse");

        match command {
            Command::Serve {
                allowed_origins, ..
            } => {
                assert_eq!(
                    allowed_origins,
                    vec![
                        "https://app.example".to_string(),
                        "https://admin.example".to_string()
                    ]
                );
            }
            other => assert!(matches!(other, Command::Serve { .. })),
        }
    }

    #[test]
    fn parse_serve_rejects_wildcard_allow_origin() {
        let args = vec![
            "traverse-cli".to_string(),
            "serve".to_string(),
            "--allow-origin".to_string(),
            "*".to_string(),
        ];

        let error = parse_command(&args).expect_err("wildcard origin should be rejected");
        assert!(error.contains("--allow-origin '*' is not allowed"));
    }

    #[test]
    fn parse_serve_accepts_qr_flag() {
        let args = vec![
            "traverse-cli".to_string(),
            "serve".to_string(),
            "--qr".to_string(),
        ];

        let command = parse_command(&args).expect("serve command should parse");

        match command {
            Command::Serve {
                render_mobile_qr, ..
            } => assert!(render_mobile_qr),
            other => assert!(matches!(other, Command::Serve { .. })),
        }
    }

    #[test]
    fn parse_serve_dev_any_defaults_to_wildcard_bind() {
        let args = vec![
            "traverse-cli".to_string(),
            "serve".to_string(),
            "--auth".to_string(),
            "dev-any".to_string(),
        ];

        let command = parse_command(&args).expect("serve command should parse");

        match command {
            Command::Serve {
                bind_address,
                auth_mode,
                ..
            } => {
                assert_eq!(bind_address, "0.0.0.0:8787");
                assert_eq!(auth_mode, Some("dev-any".to_string()));
            }
            other => assert!(matches!(other, Command::Serve { .. })),
        }
    }

    #[test]
    fn parse_serve_dev_any_port_uses_wildcard_bind() {
        let args = vec![
            "traverse-cli".to_string(),
            "serve".to_string(),
            "--auth".to_string(),
            "dev-any".to_string(),
            "--port".to_string(),
            "9090".to_string(),
        ];

        let command = parse_command(&args).expect("serve command should parse");

        match command {
            Command::Serve { bind_address, .. } => {
                assert_eq!(bind_address, "0.0.0.0:9090");
            }
            other => assert!(matches!(other, Command::Serve { .. })),
        }
    }

    #[test]
    fn parse_serve_rejects_unknown_auth_mode() {
        let args = vec![
            "traverse-cli".to_string(),
            "serve".to_string(),
            "--auth".to_string(),
            "token".to_string(),
        ];

        let error = parse_command(&args).expect_err("unsupported auth mode should be rejected");
        assert!(error.contains("--auth value must be dev-loopback or dev-any"));
    }

    #[test]
    fn parse_command_rejects_unknown_shape() {
        let args = vec!["traverse-cli".to_string()];
        let result = parse_command(&args);
        assert!(result.is_err());
        let error = result.err().unwrap_or_default();
        assert!(error.contains("usage: traverse-cli"));
    }

    #[test]
    fn parse_command_returns_bundle_inspect_help_on_help_flag() {
        let args = vec![
            "traverse-cli".to_string(),
            "bundle".to_string(),
            "inspect".to_string(),
            "--help".to_string(),
        ];
        let result = parse_command(&args);
        assert!(result.is_err(), "expected Err for --help");
        let text = result.err().unwrap_or_default();
        assert!(
            text.contains("bundle inspect"),
            "expected 'bundle inspect' in help text"
        );
        assert!(
            text.contains("<manifest-path>"),
            "expected '<manifest-path>' in help text"
        );
        assert!(
            text.contains("Example:"),
            "expected 'Example:' in help text"
        );
    }

    #[test]
    fn parse_command_returns_bundle_register_help_on_help_flag() {
        let args = vec![
            "traverse-cli".to_string(),
            "bundle".to_string(),
            "register".to_string(),
            "--help".to_string(),
        ];
        let result = parse_command(&args);
        assert!(result.is_err(), "expected Err for --help");
        let text = result.err().unwrap_or_default();
        assert!(text.contains("bundle register"));
        assert!(text.contains("<manifest-path>"));
        assert!(text.contains("Example:"));
    }

    #[test]
    fn parse_command_returns_capability_package_inspect_help_on_help_flag() {
        let args = vec![
            "traverse-cli".to_string(),
            "capability-package".to_string(),
            "inspect".to_string(),
            "--help".to_string(),
        ];
        let result = parse_command(&args);
        assert!(result.is_err(), "expected Err for --help");
        let text = result.err().unwrap_or_default();
        assert!(text.contains("capability-package inspect"));
        assert!(text.contains("<manifest-path>"));
        assert!(text.contains("Example:"));
    }

    #[test]
    fn parse_command_returns_capability_package_execute_help_on_help_flag() {
        let args = vec![
            "traverse-cli".to_string(),
            "capability-package".to_string(),
            "execute".to_string(),
            "--help".to_string(),
        ];
        let result = parse_command(&args);
        assert!(result.is_err(), "expected Err for --help");
        let text = result.err().unwrap_or_default();
        assert!(text.contains("capability-package execute"));
        assert!(text.contains("<manifest-path>"));
        assert!(text.contains("<request-path>"));
        assert!(text.contains("Example:"));
    }

    #[test]
    fn parse_command_returns_artifact_verify_help_on_help_flag() {
        let args = vec![
            "traverse-cli".to_string(),
            "artifact".to_string(),
            "verify".to_string(),
            "--help".to_string(),
        ];
        let result = parse_command(&args);
        assert!(result.is_err(), "expected Err for --help");
        let text = result.err().unwrap_or_default();
        assert!(text.contains("artifact verify"));
        assert!(text.contains("<artifact-or-manifest-path>"));
        assert!(text.contains("Example:"));
    }

    #[test]
    fn parse_command_returns_artifact_sign_help_on_help_flag() {
        let args = vec![
            "traverse-cli".to_string(),
            "artifact".to_string(),
            "sign".to_string(),
            "--help".to_string(),
        ];
        let result = parse_command(&args);
        assert!(result.is_err(), "expected Err for --help");
        let text = result.err().unwrap_or_default();
        assert!(text.contains("artifact sign"));
        assert!(text.contains("<artifact-path>"));
        assert!(text.contains("Example:"));
    }

    #[test]
    fn parse_command_parses_artifact_sign() {
        let args = vec![
            "traverse-cli".to_string(),
            "artifact".to_string(),
            "sign".to_string(),
            "target/release/traverse-cli".to_string(),
        ];
        let result = parse_command(&args);
        assert!(matches!(
            result,
            Ok(Command::ArtifactSign { artifact_path })
                if artifact_path == Path::new("target/release/traverse-cli")
        ));
    }

    #[test]
    fn parse_command_rejects_artifact_sign_with_wrong_arity() {
        let args = vec![
            "traverse-cli".to_string(),
            "artifact".to_string(),
            "sign".to_string(),
        ];
        assert!(parse_command(&args).is_err());
    }

    #[test]
    fn artifact_sign_then_verify_round_trips_through_run_command() {
        let dir = unique_temp_dir();
        let artifact = dir.join("artifact.bin");
        fs::write(&artifact, b"cli round trip bytes").expect("artifact should write");

        let signed = run_command(Command::ArtifactSign {
            artifact_path: artifact.clone(),
        })
        .expect("signing a real artifact must succeed");
        assert!(signed.contains("\"signing_scheme\": \"ed25519\""));

        let verified = run_command(Command::ArtifactVerify {
            artifact_path: artifact.clone(),
        });
        // Checksum and signature must verify even without a provenance
        // sidecar; overall status stays Failed only because provenance is
        // absent, which is expected and separate from what this command
        // signs.
        let report_json = verified
            .or_else(|error| match error {
                CliError::ValidationFailed(json) => Ok(json),
                other => Err(other),
            })
            .expect("verifying a signed artifact should only fail on missing provenance");
        assert!(report_json.contains("\"signature_status\": \"verified\""));
        assert!(report_json.contains("\"checksum_status\": \"matched\""));
    }

    #[test]
    fn parse_command_returns_workflow_inspect_help_on_help_flag() {
        let args = vec![
            "traverse-cli".to_string(),
            "workflow".to_string(),
            "inspect".to_string(),
            "--help".to_string(),
        ];
        let result = parse_command(&args);
        assert!(result.is_err(), "expected Err for --help");
        let text = result.err().unwrap_or_default();
        assert!(text.contains("workflow inspect"));
        assert!(text.contains("<workflow-id>"));
        assert!(text.contains("Example:"));
    }

    #[test]
    fn parse_command_returns_expedition_execute_help_on_help_flag() {
        let args = vec![
            "traverse-cli".to_string(),
            "expedition".to_string(),
            "execute".to_string(),
            "--help".to_string(),
        ];
        let result = parse_command(&args);
        assert!(result.is_err(), "expected Err for --help");
        let text = result.err().unwrap_or_default();
        assert!(text.contains("expedition execute"));
        assert!(text.contains("<request-path>"));
        assert!(text.contains("--trace-out"));
        assert!(text.contains("Example:"));
    }

    #[test]
    fn parse_command_returns_capability_inspect_help_on_help_flag() {
        let args = vec![
            "traverse-cli".to_string(),
            "capability".to_string(),
            "inspect".to_string(),
            "--help".to_string(),
        ];
        let result = parse_command(&args);
        assert!(result.is_err(), "expected Err for --help");
        let text = result.err().unwrap_or_default();
        assert!(text.contains("capability inspect"));
        assert!(text.contains("<contract-path>"));
        assert!(text.contains("Example:"));
    }

    #[test]
    fn parse_command_returns_event_inspect_help_on_help_flag() {
        let args = vec![
            "traverse-cli".to_string(),
            "event".to_string(),
            "inspect".to_string(),
            "--help".to_string(),
        ];
        let result = parse_command(&args);
        assert!(result.is_err(), "expected Err for --help");
        let text = result.err().unwrap_or_default();
        assert!(text.contains("event inspect"));
        assert!(text.contains("<contract-path>"));
        assert!(text.contains("Example:"));
    }

    #[test]
    fn parse_command_returns_trace_inspect_help_on_help_flag() {
        let args = vec![
            "traverse-cli".to_string(),
            "trace".to_string(),
            "inspect".to_string(),
            "--help".to_string(),
        ];
        let result = parse_command(&args);
        assert!(result.is_err(), "expected Err for --help");
        let text = result.err().unwrap_or_default();
        assert!(text.contains("trace inspect"));
        assert!(text.contains("<trace-path>"));
        assert!(text.contains("Example:"));
    }

    #[test]
    fn serve_help_marks_http_discovery_as_development_and_ci_only() {
        let text = help_serve();
        assert!(text.contains("development and CI HTTP/JSON API"));
        assert!(text.contains("not the production app topology"));
        assert!(text.contains("neither a loopback sidecar"));
        assert!(text.contains(".traverse/server.json discovery"));
    }

    #[test]
    fn parse_command_returns_family_help_when_only_family_and_help_flag() {
        let cases = vec![
            (vec!["traverse-cli", "bundle", "--help"], "bundle"),
            (
                vec!["traverse-cli", "capability-package", "--help"],
                "capability-package",
            ),
            (vec!["traverse-cli", "workflow", "--help"], "workflow"),
            (vec!["traverse-cli", "expedition", "--help"], "expedition"),
            (vec!["traverse-cli", "event", "--help"], "event"),
            (vec!["traverse-cli", "trace", "--help"], "trace"),
        ];
        for (raw, expected_family) in cases {
            let args: Vec<String> = raw.into_iter().map(String::from).collect();
            let result = parse_command(&args);
            assert!(
                result.is_err(),
                "expected Err for --help on family {expected_family}"
            );
            let text = result.err().unwrap_or_default();
            assert!(
                text.contains(expected_family),
                "expected '{expected_family}' in family help text"
            );
        }
    }

    #[test]
    fn inspect_bundle_renders_canonical_example_bundle() {
        let manifest_path = repo_root().join("examples/expedition/registry-bundle/manifest.json");

        let output = inspect_bundle(&manifest_path, false).expect("bundle inspect should succeed");

        assert!(output.contains("bundle_id: expedition.planning.seed-bundle"));
        assert!(output.contains("event_ids:"));
        assert!(output.contains("workflow_ids:"));
    }

    #[test]
    fn inspect_bundle_rejects_missing_artifact_paths() {
        let temp_dir = unique_temp_dir();
        let manifest_path = temp_dir.join("manifest.json");
        fs::write(
            &manifest_path,
            r#"{
  "bundle_id": "expedition.planning.seed-bundle",
  "version": "1.0.0",
  "scope": "public",
  "capabilities": [
    {
      "id": "expedition.planning.capture-expedition-objective",
      "version": "1.0.0",
      "path": "missing/capability.json"
    }
  ],
  "events": [],
  "workflows": []
}"#,
        )
        .expect("manifest should write");

        let error =
            inspect_bundle(&manifest_path, false).expect_err("missing artifact path should fail");
        assert!(error.message().contains("missing artifact file"));
    }

    #[test]
    fn register_bundle_registers_canonical_expedition_artifacts() {
        let manifest_path = repo_root().join("examples/expedition/registry-bundle/manifest.json");

        let registered = load_registered_bundle(&manifest_path)
            .expect("non-shadowing private bundle should register");
        assert!(registered.evidence.is_empty());
        let output =
            register_bundle(&manifest_path, false).expect("bundle register should succeed");

        assert!(output.contains("registered_capabilities: 6"));
        assert!(output.contains("registered_events: 5"));
        assert!(output.contains("registered_workflows: 1"));
        assert!(output.contains("expedition.planning.plan-expedition@1.0.0 (workflow)"));
    }

    #[test]
    fn capability_discover_json_distinguishes_package_mode_from_activation_eligibility() {
        let manifest_path =
            repo_root().join("examples/traverse-starter/registry-bundle/manifest.json");
        let output = discover_capabilities(&manifest_path, true)
            .expect("capability discovery should render JSON");
        let entries: Vec<Value> =
            serde_json::from_str(&output).expect("discovery output should be JSON");
        let process = entries
            .iter()
            .find(|entry| entry["id"] == "traverse-starter.process")
            .expect("process capability should be discovered");
        let pipeline = entries
            .iter()
            .find(|entry| entry["id"] == "traverse-starter.pipeline")
            .expect("pipeline capability should be discovered");

        assert_eq!(process["package_mode"], "standalone");
        assert_eq!(process["advisory_compositions"], serde_json::json!([]));
        assert_eq!(process["activation_eligibility"], "unknown");
        assert_eq!(
            process["activation_eligibility_reason"],
            "requires_host_activation_resolution"
        );
        assert_eq!(pipeline["package_mode"], "workflow_composed");
        assert_eq!(
            pipeline["advisory_compositions"],
            serde_json::json!(["traverse-starter.pipeline@1.0.0"])
        );
        assert_eq!(pipeline["activation_eligibility"], "unknown");
    }

    #[test]
    fn register_bundle_rejects_local_public_scope_with_stable_guidance() {
        let source = repo_root().join("examples/expedition/registry-bundle/manifest.json");
        let source_parent = source.parent().expect("bundle manifest must have parent");
        let mut manifest: Value = serde_json::from_str(
            &fs::read_to_string(&source).expect("canonical bundle manifest should read"),
        )
        .expect("canonical bundle manifest should parse");
        manifest["scope"] = Value::String("public".to_string());
        for collection in ["capabilities", "events", "workflows"] {
            for artifact in manifest[collection]
                .as_array_mut()
                .expect("artifact collection should be an array")
            {
                let relative = artifact["path"]
                    .as_str()
                    .expect("artifact path should be a string");
                artifact["path"] =
                    Value::String(source_parent.join(relative).display().to_string());
            }
        }
        let temp_dir = unique_temp_dir();
        let manifest_path = temp_dir.join("manifest.json");
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).expect("manifest should serialize"),
        )
        .expect("manifest should write");

        let error = load_registered_bundle(&manifest_path)
            .expect_err("local public bundle registration must fail");

        assert!(error.message().contains("local_public_scope_rejected"));
        assert!(error.message().contains("scope: private"));
        assert!(error.message().contains("capability publish"));
    }

    #[test]
    fn private_bundle_shadow_emits_evidence_and_keeps_private_precedence() {
        let manifest_path =
            repo_root().join("examples/traverse-starter/registry-bundle/manifest.json");
        let public = PublicRegistryCapabilityRecord {
            namespace: "traverse-starter".to_string(),
            id: "traverse-starter.process".to_string(),
            version: "1.0.0".to_string(),
            digest: "sha256:fixture".to_string(),
            artifact_url: "https://example.invalid/process.wasm".to_string(),
            contract_digest: "sha256:contract".to_string(),
            contract_url: "https://example.invalid/contract.json".to_string(),
            deprecated: false,
        };

        let registered = load_registered_bundle_with_public_records(&manifest_path, &[public])
            .expect("private shadow registration should succeed");

        assert_eq!(registered.evidence.len(), 1);
        assert_eq!(registered.evidence[0].code, "private_shadows_synced_public");
        let resolved = registered
            .capability_registry
            .find_exact(
                traverse_registry::LookupScope::PreferPrivate,
                "traverse-starter.process",
                "1.0.0",
            )
            .expect("prefer-private lookup should resolve local shadow");
        assert_eq!(
            resolved.record.scope,
            traverse_registry::RegistryScope::Private
        );
    }

    #[test]
    fn register_bundle_rejects_duplicate_manifest_entries() {
        let temp_dir = unique_temp_dir();
        let manifest_path = temp_dir.join("manifest.json");
        fs::write(
            &manifest_path,
            r#"{
  "bundle_id": "expedition.planning.seed-bundle",
  "version": "1.0.0",
  "scope": "private",
  "capabilities": [
    {
      "id": "expedition.planning.capture-expedition-objective",
      "version": "1.0.0",
      "path": "../../../contracts/examples/expedition/capabilities/capture-expedition-objective/contract.json"
    },
    {
      "id": "expedition.planning.capture-expedition-objective",
      "version": "1.0.0",
      "path": "../../../contracts/examples/expedition/capabilities/capture-expedition-objective/contract.json"
    }
  ],
  "events": [],
  "workflows": []
}"#,
        )
        .expect("manifest should write");

        let error = register_bundle(&manifest_path, false)
            .expect_err("duplicate bundle entries should fail");

        assert!(
            error
                .message()
                .contains("duplicate capability artifact entry")
        );
    }

    #[test]
    fn execute_expedition_runs_canonical_plan_request() {
        let request_path =
            repo_root().join("examples/expedition/runtime-requests/plan-expedition.json");

        let output = execute_expedition(&request_path, None, false, false)
            .expect("expedition execution should succeed");

        assert!(output.contains("capability_id: expedition.planning.plan-expedition"));
        assert!(output.contains("status: completed"));
        assert!(output.contains("recommended_route_style: conservative-alpine-push"));
    }

    #[test]
    fn execute_expedition_output_changes_with_real_wasm_input_across_the_full_chain() {
        // Directly proves #916's bug is fixed: the composite workflow now
        // genuinely consults its capabilities' real WASM artifacts end to
        // end, rather than a hardcoded native reimplementation that would
        // ignore the request and always emit the same fabricated values.
        let canonical_path =
            repo_root().join("examples/expedition/runtime-requests/plan-expedition.json");
        let mut request: Value = serde_json::from_str(
            &fs::read_to_string(&canonical_path).expect("canonical request must read"),
        )
        .expect("canonical request must parse");
        request["input"]["destination"] = Value::String("Mount Rainier".to_string());
        request["request_id"] = Value::String("expedition-plan-request-mount-rainier".to_string());

        let temp_dir = unique_temp_dir();
        let request_path = temp_dir.join("mount-rainier-request.json");
        fs::write(
            &request_path,
            serde_json::to_string_pretty(&request).expect("request must serialize"),
        )
        .expect("request must write");

        let output = execute_expedition(&request_path, None, false, false)
            .expect("expedition execution should succeed");

        assert!(output.contains("status: completed"));
        assert!(
            output.contains("objective_id: objective-mountrainier"),
            "objective_id must be derived from the real destination the WASM artifact received, not a fixed value; got: {output}"
        );
        assert!(
            output.contains("plan_id: plan-objective-mountrainier"),
            "plan_id must reflect the same real, request-derived objective_id all the way through the chain; got: {output}"
        );
    }

    #[test]
    fn expedition_execute_help_discloses_execution_honesty() {
        let help = help_expedition_execute();

        assert!(
            help.contains("ArtifactRouter"),
            "help text must disclose that capabilities run through the real WASM execution path"
        );
        assert!(
            help.contains("capability-package execute"),
            "help text must name the shared execution path this command also uses"
        );
    }

    #[test]
    fn execute_expedition_rejects_a_capability_not_in_the_canonical_bundle() {
        let temp_dir = unique_temp_dir();
        let path = temp_dir.join("unknown-capability-request.json");
        fs::write(
            &path,
            r#"{
  "kind": "runtime_request",
  "schema_version": "1.0.0",
  "request_id": "unknown-capability-request",
  "intent": {
    "capability_id": "expedition.planning.not-a-real-capability",
    "capability_version": "1.0.0"
  },
  "input": {},
  "lookup": {
    "scope": "prefer_private",
    "allow_ambiguity": false
  },
  "context": {
    "requested_target": "local"
  },
  "governing_spec": "006-runtime-request-execution"
}"#,
        )
        .expect("runtime request should write");

        let error = execute_expedition(&path, None, false, false)
            .expect_err("unregistered capability id must fail closed");

        assert!(error.message().contains("runtime execution failed"));
    }

    #[test]
    fn inspect_capability_package_renders_governed_wasm_capability_package() {
        let fixture = create_interpret_expedition_intent_capability_fixture();

        let output = inspect_capability_package(&fixture.manifest_path)
            .expect("capability-package inspect should succeed");

        assert!(output.contains("package_id: expedition.planning.interpret-expedition-intent"));
        assert!(output.contains("capability_id: expedition.planning.interpret-expedition-intent"));
        assert!(output.contains("binary_digest: fnv1a64:"));
        assert!(output.contains("known_compositions: expedition.planning.plan-expedition@1.0.0"));
    }

    #[test]
    fn execute_capability_package_runs_non_hardcoded_wasm_package() {
        let manifest_path = repo_root().join("examples/doc-approval/analyze-agent/manifest.json");
        let request_path = repo_root().join("examples/doc-approval/runtime-requests/analyze.json");

        let output = execute_capability_package(&manifest_path, &request_path)
            .expect("real doc-approval WASM execution should succeed");

        assert!(output.contains("capability_id: doc-approval.analyze"));
        assert!(output.contains("capability_version: 1.0.0"));
        assert!(output.contains("status: completed"));
        assert!(output.contains("output:"));
        assert!(output.contains("\"recommendation\": \"manual_review\""));
    }

    #[test]
    fn format_capability_package_execution_summary_uses_manifest_version_and_json_output() {
        let rendered = format_capability_package_execution_summary(
            "example.package",
            "example.capability",
            "2.3.4",
            "req-1",
            "exec-1",
            "trace-1",
            &serde_json::json!({
                "greeting": "Hello",
                "nested": {"ok": true}
            }),
        );

        assert!(rendered.contains("capability_version: 2.3.4"));
        assert!(rendered.contains("package_id: example.package"));
        assert!(rendered.contains("capability_id: example.capability"));
        assert!(rendered.contains("request_id: req-1"));
        assert!(rendered.contains("execution_id: exec-1"));
        assert!(rendered.contains("trace_ref: trace-1"));
        assert!(rendered.contains("status: completed"));
        assert!(rendered.contains("output:"));
        assert!(rendered.contains("\"greeting\": \"Hello\""));
        assert!(rendered.contains("\"ok\": true"));

        let null_output = format_capability_package_execution_summary(
            "example.package",
            "example.capability",
            "9.9.9",
            "req-2",
            "exec-2",
            "trace-2",
            &Value::Null,
        );
        assert!(null_output.contains("capability_version: 9.9.9"));
        assert!(null_output.contains("output:\nnull"));
    }

    #[derive(Default)]
    struct SpyTelemetrySink {
        events: Arc<Mutex<Vec<UsageEvent>>>,
    }

    impl UsageTelemetrySink for SpyTelemetrySink {
        fn record(&self, event: UsageEvent) {
            self.events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(event);
        }
    }

    #[test]
    fn execute_with_telemetry_fires_exactly_one_execute_event_on_real_success() {
        let manifest_path = repo_root().join("examples/doc-approval/analyze-agent/manifest.json");
        let request_path = repo_root().join("examples/doc-approval/runtime-requests/analyze.json");

        let package =
            load_capability_package(&manifest_path).expect("doc-approval package must load");
        let request = load_runtime_request(&request_path).expect("request must load");
        let mut registry = CapabilityRegistry::new();
        registry
            .register(package.capability_registration())
            .expect("capability must register");
        let runtime = Runtime::new(
            registry,
            ArtifactRouter::new().expect("artifact router must construct"),
        )
        .with_security_config(traverse_runtime::security::RuntimeSecurityConfig::development());

        let sink = SpyTelemetrySink::default();
        let events = sink.events.clone();
        let outcome = telemetry::execute_with_telemetry(&runtime, request, &sink);

        assert_eq!(outcome.result.status, RuntimeResultStatus::Completed);
        let recorded = events.lock().expect("lock must not be poisoned");
        assert_eq!(
            recorded.len(),
            1,
            "a successful real WASM invocation must fire exactly one execute event"
        );
        assert_eq!(recorded[0].kind, UsageEventKind::Execute);
        assert_eq!(recorded[0].capability_ref, "doc-approval.analyze@1.0.0");
    }

    #[test]
    fn execute_capability_package_runs_governed_capability_package_request() {
        let manifest_path =
            repo_root().join("examples/capabilities/expedition-intent-agent/manifest.json");
        build_real_capability_package_artifact(&manifest_path);
        let request_path = repo_root()
            .join("examples/capabilities/runtime-requests/interpret-expedition-intent.json");

        let output = execute_capability_package(&manifest_path, &request_path)
            .expect("capability package execution should succeed");

        assert!(output.contains("package_id: expedition.planning.interpret-expedition-intent"));
        assert!(output.contains("capability_id: expedition.planning.interpret-expedition-intent"));
        assert!(output.contains("capability_version: 1.0.0"));
        assert!(output.contains("status: completed"));
        assert!(output.contains("\"conservative-alpine-push\""));
        assert!(output.contains("\"same-day-return\""));
    }

    #[test]
    fn inspect_capability_package_renders_second_governed_wasm_capability_package() {
        let fixture = create_validate_team_readiness_capability_fixture();

        let output = inspect_capability_package(&fixture.manifest_path)
            .expect("capability-package inspect should succeed");

        assert!(output.contains("package_id: expedition.planning.validate-team-readiness"));
        assert!(output.contains("capability_id: expedition.planning.validate-team-readiness"));
        assert!(output.contains("binary_digest: fnv1a64:"));
        assert!(output.contains("known_compositions: expedition.planning.plan-expedition@1.0.0"));
    }

    #[test]
    fn execute_capability_package_runs_second_governed_capability_package_request() {
        let manifest_path =
            repo_root().join("examples/capabilities/team-readiness-agent/manifest.json");
        build_real_capability_package_artifact(&manifest_path);
        let request_path =
            repo_root().join("examples/capabilities/runtime-requests/validate-team-readiness.json");

        let output = execute_capability_package(&manifest_path, &request_path)
            .expect("capability package execution should succeed");

        assert!(output.contains("package_id: expedition.planning.validate-team-readiness"));
        assert!(output.contains("capability_id: expedition.planning.validate-team-readiness"));
        assert!(output.contains("capability_version: 1.0.0"));
        assert!(output.contains("status: completed"));
        assert!(output.contains("\"status\": \"ready\"") || output.contains("\"ready\""));
    }

    #[test]
    fn inspect_capability_package_renders_hello_world_package() {
        let fixture = create_hello_world_capability_fixture();

        let output = inspect_capability_package(&fixture.manifest_path)
            .expect("capability-package inspect should succeed");

        assert!(output.contains("package_id: hello.world.say-hello-agent"));
        assert!(output.contains("capability_id: hello.world.say-hello"));
        assert!(output.contains("binary_digest: fnv1a64:"));
        assert!(output.contains("known_compositions: hello.world.say-hello@1.0.0"));
    }

    #[test]
    fn execute_capability_package_runs_hello_world_request() {
        let manifest_path = repo_root().join("examples/hello-world/say-hello-agent/manifest.json");
        build_real_capability_package_artifact(&manifest_path);
        let request_path = repo_root().join("examples/hello-world/runtime-requests/say-hello.json");

        let output = execute_capability_package(&manifest_path, &request_path)
            .expect("hello-world capability package execution should succeed");

        assert!(output.contains("package_id: hello.world.say-hello-agent"));
        assert!(output.contains("capability_id: hello.world.say-hello"));
        assert!(output.contains("capability_version: 1.0.0"));
        assert!(output.contains("status: completed"));
        assert!(output.contains("\"name\": \"Traverse\""));
        assert!(output.contains("\"greeting\": \"Hello, Traverse!\""));
    }

    #[test]
    fn execute_capability_package_runs_traverse_starter_process_request() {
        let manifest_path =
            repo_root().join("examples/traverse-starter/process-agent/manifest.json");
        build_real_capability_package_artifact(&manifest_path);
        let request_path =
            repo_root().join("examples/traverse-starter/runtime-requests/process.json");

        let output = execute_capability_package(&manifest_path, &request_path)
            .expect("traverse-starter capability package execution should succeed");

        assert!(output.contains("package_id: traverse-starter.process-agent"));
        assert!(output.contains("capability_id: traverse-starter.process"));
        assert!(output.contains("capability_version: 1.0.0"));
        assert!(output.contains("status: completed"));
        assert!(output.contains("\"title\": \"Review Traverse starter app registration\""));
        assert!(output.contains("\"noteType\": \"project\""));
        assert!(output.contains("\"suggestedNextAction\": \"expand\""));
        assert!(output.contains("\"status\": \"complete\""));
    }

    #[test]
    fn execute_capability_package_runs_traverse_starter_validate_request() {
        let manifest_path =
            repo_root().join("examples/traverse-starter/validate-agent/manifest.json");
        build_real_capability_package_artifact(&manifest_path);
        let request_path =
            repo_root().join("examples/traverse-starter/runtime-requests/validate.json");

        let output = execute_capability_package(&manifest_path, &request_path)
            .expect("traverse-starter validate capability package execution should succeed");

        assert!(output.contains("package_id: traverse-starter.validate-agent"));
        assert!(output.contains("capability_id: traverse-starter.validate"));
        assert!(output.contains("capability_version: 1.0.0"));
        assert!(output.contains("status: completed"));
        assert!(output.contains("\"valid\": true"));
        assert!(output.contains("\"issues\""));
    }

    #[test]
    fn execute_capability_package_runs_traverse_starter_summarize_request() {
        let manifest_path =
            repo_root().join("examples/traverse-starter/summarize-agent/manifest.json");
        build_real_capability_package_artifact(&manifest_path);
        let request_path =
            repo_root().join("examples/traverse-starter/runtime-requests/summarize.json");

        let output = execute_capability_package(&manifest_path, &request_path)
            .expect("traverse-starter summarize capability package execution should succeed");

        assert!(output.contains("package_id: traverse-starter.summarize-agent"));
        assert!(output.contains("capability_id: traverse-starter.summarize"));
        assert!(output.contains("capability_version: 1.0.0"));
        assert!(output.contains("status: completed"));
        assert!(output.contains(
            "Review Traverse starter app (project) - tags: review, traverse, starter; next action: expand"
        ));
        assert!(output.contains("\"wordCount\": 13"));
    }

    #[test]
    fn traverse_starter_pipeline_bundle_executes_with_merged_namespaced_output() {
        let bundle_path =
            repo_root().join("examples/traverse-starter/registry-bundle/manifest.json");
        let request_path =
            repo_root().join("examples/traverse-starter/runtime-requests/pipeline.json");

        let registered =
            load_registered_bundle(&bundle_path).expect("starter bundle should register");
        // Development security mode mirrors the CLI's own execution paths: the
        // starter bundle ships unsigned local-dev artifacts, which the default
        // production posture rejects on every workflow step (spec 030 FR-013).
        let runtime = traverse_runtime::Runtime::new(
            registered.capability_registry,
            ExpeditionExampleExecutor,
        )
        .with_workflow_registry(registered.workflow_registry)
        .with_security_config(traverse_runtime::security::RuntimeSecurityConfig::development());

        let first_request =
            load_runtime_request(&request_path).expect("pipeline request should load");
        let second_request =
            load_runtime_request(&request_path).expect("pipeline request should load");
        let first = runtime.execute(first_request);
        let second = runtime.execute(second_request);

        assert_eq!(
            first.result.status,
            traverse_runtime::RuntimeResultStatus::Completed
        );
        let output = first
            .result
            .output
            .clone()
            .expect("pipeline output expected");
        assert_eq!(
            output,
            serde_json::json!({
                "validate": {"valid": true, "issues": []},
                "process": {
                    "title": "Review Traverse starter app registration",
                    "tags": ["review", "traverse", "starter"],
                    "noteType": "project",
                    "suggestedNextAction": "expand",
                    "status": "complete"
                },
                "summarize": {
                    "summary": "Review Traverse starter app registration (project) - tags: review, traverse, starter; next action: expand",
                    "wordCount": 14
                }
            })
        );
        assert_eq!(first.result.output, second.result.output);
        assert_eq!(first.result.status, second.result.status);
    }

    #[test]
    fn traverse_starter_validate_rejects_empty_note_deterministically() {
        let input = serde_json::json!({ "note": "   " });

        let first = execute_traverse_starter_validate(&input)
            .expect("validate should succeed for empty note");
        let second = execute_traverse_starter_validate(&input)
            .expect("validate should succeed for empty note");

        assert_eq!(first, second);
        assert_eq!(first["valid"], serde_json::json!(false));
        let issues = first["issues"]
            .as_array()
            .expect("issues should be an array");
        assert!(!issues.is_empty());
        assert_eq!(issues[0], "note must not be empty");
    }

    #[test]
    fn traverse_starter_validate_accepts_valid_note_deterministically() {
        let input = serde_json::json!({ "note": "Review Traverse starter app registration path" });

        let first =
            execute_traverse_starter_validate(&input).expect("validate should succeed for note");
        let second =
            execute_traverse_starter_validate(&input).expect("validate should succeed for note");

        assert_eq!(first, second);
        assert_eq!(first, serde_json::json!({ "valid": true, "issues": [] }));
    }

    #[test]
    fn traverse_starter_validate_flags_note_over_max_length() {
        let input = serde_json::json!({ "note": "n".repeat(2001) });

        let output =
            execute_traverse_starter_validate(&input).expect("validate should succeed for note");

        assert_eq!(output["valid"], serde_json::json!(false));
        assert_eq!(
            output["issues"],
            serde_json::json!(["note must be at most 2000 characters"])
        );
    }

    #[test]
    fn traverse_starter_summarize_produces_deterministic_summary_from_process_output() {
        let note_input =
            serde_json::json!({ "note": "Review Traverse starter app registration path" });
        let process_output =
            execute_traverse_starter_process(&note_input).expect("process should succeed for note");

        let first = execute_traverse_starter_summarize(&process_output)
            .expect("summarize should succeed for process output");
        let second = execute_traverse_starter_summarize(&process_output)
            .expect("summarize should succeed for process output");

        assert_eq!(first, second);
        let summary = first["summary"]
            .as_str()
            .expect("summary should be a string");
        assert!(summary.contains("Review Traverse starter app"));
        assert!(summary.contains("(project)"));
        assert!(summary.contains("next action: expand"));
        let word_count = first["wordCount"]
            .as_u64()
            .expect("wordCount should be a number");
        assert_eq!(word_count, summary.split_whitespace().count() as u64);
    }

    #[test]
    fn traverse_starter_summarize_reports_missing_tags_as_none() {
        let input = serde_json::json!({
            "title": "Untitled note",
            "tags": [],
            "noteType": "fleeting",
            "suggestedNextAction": "archive"
        });

        let output = execute_traverse_starter_summarize(&input)
            .expect("summarize should succeed for minimal input");

        assert_eq!(
            output["summary"],
            serde_json::json!("Untitled note (fleeting) - tags: none; next action: archive")
        );
    }

    #[test]
    fn execute_capability_package_runs_meeting_notes_process_request() {
        let manifest_path = repo_root().join("examples/meeting-notes/process-agent/manifest.json");
        build_real_capability_package_artifact(&manifest_path);
        let request_path = repo_root().join("examples/meeting-notes/runtime-requests/process.json");

        let output = execute_capability_package(&manifest_path, &request_path)
            .expect("meeting-notes capability package execution should succeed");

        assert!(output.contains("package_id: meeting-notes.process-agent"));
        assert!(output.contains("capability_id: meeting-notes.process"));
        assert!(output.contains("capability_version: 1.0.0"));
        assert!(output.contains("status: completed"));
        assert!(output.contains("Kickoff notes for Traverse reference app."));
        assert!(output.contains("\"action_items\""));
        assert!(output.contains("\"decisions\""));
        assert!(output.contains("\"follow_ups\""));
    }

    #[test]
    fn meeting_notes_process_is_deterministic_and_handles_empty_transcript() {
        let input = serde_json::json!({
            "transcript": "Kickoff notes.\nAction: @Mira draft parser by Friday\nDecided: we will ship deterministic extraction\nFollow up next steps with app team"
        });

        let first = crate::execute_meeting_notes_process(&input).expect("first run should succeed");
        let second =
            crate::execute_meeting_notes_process(&input).expect("second run should succeed");
        assert_eq!(first, second);
        assert_eq!(first["action_items"][0]["owner"], "Mira");
        assert_eq!(first["action_items"][0]["due"], "Friday");
        assert_eq!(
            first["decisions"][0]["text"],
            "we will ship deterministic extraction"
        );

        let empty = crate::execute_meeting_notes_process(&serde_json::json!({"transcript": ""}))
            .expect("empty transcript should succeed");
        assert_eq!(empty["summary"], "");
        assert_eq!(empty["action_items"].as_array().map(Vec::len), Some(0));
        assert_eq!(empty["decisions"].as_array().map(Vec::len), Some(0));
        assert_eq!(empty["follow_ups"].as_array().map(Vec::len), Some(0));

        let owner_only = crate::execute_meeting_notes_process(&serde_json::json!({
            "transcript": "TODO: by Luca add HTTP validation before demo"
        }))
        .expect("owner-only by marker should succeed");
        assert_eq!(owner_only["action_items"][0]["owner"], "Luca");
        assert_eq!(owner_only["action_items"][0]["due"], Value::Null);
    }

    #[test]
    fn execute_expedition_writes_trace_artifact_when_requested() {
        let request_path =
            repo_root().join("examples/expedition/runtime-requests/plan-expedition.json");
        let temp_dir = unique_temp_dir();
        let trace_path = temp_dir.join("plan-expedition-trace.json");

        let output = execute_expedition(&request_path, Some(&trace_path), false, false)
            .expect("expedition execution with trace output should succeed");

        assert!(output.contains(&format!("trace_path: {}", trace_path.display())));
        let trace_contents = fs::read_to_string(&trace_path).expect("trace file should exist");
        assert!(trace_contents.contains("\"kind\": \"runtime_trace\""));
        assert!(trace_contents.contains("\"trace_id\":"));
    }

    #[test]
    fn execute_expedition_rejects_invalid_request_input() {
        let temp_dir = unique_temp_dir();
        let path = temp_dir.join("invalid-runtime-request.json");
        fs::write(
            &path,
            r#"{
  "kind": "runtime_request",
  "schema_version": "1.0.0",
  "request_id": "invalid-expedition-plan-request",
  "intent": {
    "capability_id": "expedition.planning.plan-expedition",
    "capability_version": "1.0.0"
  },
  "input": {
    "destination": "Sky Pilot",
    "target_window": {
      "start": "2026-07-20T04:30:00Z",
      "end": "2026-07-20T16:00:00Z"
    },
    "preferences": {
      "style": "conservative-alpine-push",
      "risk_tolerance": "moderate",
      "priority": "same-day-return"
    },
    "notes": "Missing planning intent on purpose.",
    "team_profile": {
      "team_id": "team-alpine-01",
      "member_count": 3,
      "experience_level": "advanced",
      "equipment_ready": true
    }
  },
  "lookup": {
    "scope": "prefer_private",
    "allow_ambiguity": false
  },
  "context": {
    "requested_target": "local"
  },
  "governing_spec": "006-runtime-request-execution"
}"#,
        )
        .expect("runtime request should write");

        let error = execute_expedition(&path, None, false, false)
            .expect_err("invalid expedition execution should fail");

        assert!(error.message().contains("runtime execution failed"));
        assert!(
            error
                .message()
                .contains("runtime request input does not satisfy")
        );
    }

    #[test]
    fn inspect_trace_renders_generated_expedition_trace() {
        let request_path =
            repo_root().join("examples/expedition/runtime-requests/plan-expedition.json");
        let temp_dir = unique_temp_dir();
        let trace_path = temp_dir.join("plan-expedition-trace.json");

        execute_expedition(&request_path, Some(&trace_path), false, false)
            .expect("expedition execution with trace output should succeed");

        let output = inspect_trace(&trace_path).expect("trace inspect should succeed");

        assert!(output.contains("trace_id: trace_exec_expedition-plan-request-001"));
        assert!(output.contains("result_status: completed"));
        assert!(output.contains("selected_capability_id: expedition.planning.plan-expedition"));
    }

    #[test]
    fn inspect_trace_rejects_malformed_trace_artifact() {
        let temp_dir = unique_temp_dir();
        let path = temp_dir.join("trace.json");
        fs::write(&path, "{\"trace_id\":true}").expect("trace file should write");

        let error = inspect_trace(&path).expect_err("malformed trace should fail");

        assert!(error.message().contains("failed to parse runtime trace"));
    }

    #[test]
    fn inspect_event_renders_canonical_event_contract() {
        let path = repo_root().join(
            "contracts/examples/expedition/events/expedition-objective-captured/contract.json",
        );

        let output = inspect_event(&path).expect("event inspect should succeed");

        assert!(output.contains("id: expedition.planning.expedition-objective-captured"));
        assert!(output.contains("event_type: domain"));
        assert!(output.contains("publisher_ids:"));
    }

    #[test]
    fn inspect_event_rejects_malformed_contract() {
        let temp_dir = unique_temp_dir();
        let path = temp_dir.join("event.json");
        fs::write(&path, "{\"kind\":\"event_contract\"}").expect("event file should write");

        let error = inspect_event(&path).expect_err("malformed event contract should fail");

        assert!(
            error
                .message()
                .contains("failed to validate event contract")
        );
    }

    #[test]
    fn inspect_capability_renders_canonical_capability_contract() {
        let path = repo_root().join(
            "contracts/examples/expedition/capabilities/capture-expedition-objective/contract.json",
        );

        let output = inspect_capability(&path).expect("capability inspect should succeed");

        assert!(output.contains("id: expedition.planning.capture-expedition-objective"));
        assert!(output.contains("lifecycle:"));
        assert!(output.contains("input_schema_properties:"));
        assert!(output.contains("output_schema_properties:"));
        assert!(output.contains("host_api_access:"));
    }

    #[test]
    fn inspect_capability_rejects_malformed_contract() {
        let temp_dir = unique_temp_dir();
        let path = temp_dir.join("capability.json");
        fs::write(&path, "{\"kind\":\"capability_contract\"}")
            .expect("capability file should write");

        let error =
            inspect_capability(&path).expect_err("malformed capability contract should fail");

        assert!(
            error
                .message()
                .contains("failed to validate capability contract")
                || error
                    .message()
                    .contains("failed to parse capability contract")
                || error.message().contains("capability contract")
        );
    }

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn minimal_contract_json(egress_policy: traverse_contracts::EgressPolicy) -> Value {
        let risk = traverse_contracts::RiskMetadata {
            effect_class: traverse_contracts::EffectClass::ExternalEffect,
            determinism_class: traverse_contracts::DeterminismClass::ExternallyVariable,
            data_flow: traverse_contracts::DataFlowPolicy {
                egress_policy,
                ..Default::default()
            },
            reliability: traverse_contracts::ReliabilityMetadata {
                idempotency_required: true,
                retryable: false,
                compensation_available: false,
            },
        };
        serde_json::json!({
            "kind": "capability_contract",
            "schema_version": "1.0.0",
            "id": "risk.policy.example",
            "namespace": "risk.policy",
            "name": "example",
            "version": "0.1.0",
            "lifecycle": "active",
            "owner": {"team": "traverse-core", "contact": "enrico.piovesan10@gmail.com"},
            "summary": "Send a validated payload to an external system for processing.",
            "description": "Sends validated request data to an external connector for processing.",
            "inputs": {"schema": {"type": "object"}},
            "outputs": {"schema": {"type": "object"}},
            "preconditions": [],
            "postconditions": [],
            "side_effects": [{"kind": "external_call", "description": "Calls an external system."}],
            "emits": [],
            "consumes": [],
            "permissions": [],
            "execution": {
                "binary_format": "wasm",
                "entrypoint": {"kind": "wasi-command", "command": "run"},
                "preferred_targets": ["local"],
                "constraints": {
                    "host_api_access": "none",
                    "network_access": "required",
                    "filesystem_access": "none"
                }
            },
            "policies": [],
            "dependencies": [],
            "provenance": {"source": "greenfield", "author": "enricopiovesan", "created_at": "2026-08-21T00:00:00Z"},
            "evidence": [],
            "risk": risk,
        })
    }

    #[test]
    fn component_risk_policy_allows_a_subset_of_the_contract_allowlist() {
        let dir = unique_temp_dir();
        let contract_path = dir.join("contract.json");
        fs::write(
            &contract_path,
            serde_json::to_string(&minimal_contract_json(
                traverse_contracts::EgressPolicy::AllowedConnectors(vec![
                    "traverse.http".to_string(),
                ]),
            ))
            .expect("contract json should serialize"),
        )
        .expect("contract json should write");

        let component_manifest = serde_json::json!({
            "contract_path": "contract.json",
            "risk_policy": {"egress_allowed_connectors": ["traverse.http"]}
        });
        let component_path = dir.join("component.manifest.json");

        let result = validate_component_risk_policy_for_cli(&component_path, &component_manifest);
        assert_eq!(result, None);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn component_risk_policy_rejects_a_connector_beyond_the_contract_allowlist() {
        let dir = unique_temp_dir();
        let contract_path = dir.join("contract.json");
        fs::write(
            &contract_path,
            serde_json::to_string(&minimal_contract_json(
                traverse_contracts::EgressPolicy::AllowedConnectors(vec![
                    "traverse.http".to_string(),
                ]),
            ))
            .expect("contract json should serialize"),
        )
        .expect("contract json should write");

        let component_manifest = serde_json::json!({
            "contract_path": "contract.json",
            "risk_policy": {
                "egress_allowed_connectors": ["traverse.http", "traverse.object-store"]
            }
        });
        let component_path = dir.join("component.manifest.json");

        let result = validate_component_risk_policy_for_cli(&component_path, &component_manifest);
        assert_eq!(
            result,
            Some(AppValidationError {
                code: "risk_policy_weakened".to_string(),
                path: format!(
                    "{}:$.risk_policy.egress_allowed_connectors",
                    component_path.display()
                ),
                message: "manifest allows connector 'traverse.object-store' the contract's \
                          egress policy does not permit"
                    .to_string(),
            })
        );

        let _ = fs::remove_dir_all(&dir);
    }

    fn unique_temp_dir() -> PathBuf {
        static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(1);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let sequence = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("traverse-cli-test-{nanos}-{sequence}"));
        fs::create_dir_all(&path).expect("temporary directory should create");
        path
    }

    struct CapabilityPublishFixture {
        contract: PathBuf,
        artifact: PathBuf,
        registry_repo: PathBuf,
    }

    impl CapabilityPublishFixture {
        fn request(&self, dry_run: bool) -> CapabilityPublishRequest {
            CapabilityPublishRequest {
                contract_path: self.contract.clone(),
                artifact_path: self.artifact.clone(),
                registry_repo_path: self.registry_repo.clone(),
                registry_repo_remote: None,
                json_output: true,
                dry_run,
            }
        }

        fn registry_contract_path(&self) -> PathBuf {
            self.registry_repo
                .join("capabilities/traverse-starter/traverse-starter.process/1.0.0/contract.json")
        }
    }

    #[derive(Default)]
    struct RecordingPublishRunner {
        fail_program: Option<&'static str>,
        fail_command_prefix: Option<&'static str>,
        commands: RefCell<Vec<String>>,
    }

    impl PublishProcessRunner for RecordingPublishRunner {
        fn run(
            &self,
            cwd: &Path,
            program: &str,
            args: &[String],
        ) -> Result<PublishCommandOutput, String> {
            let rendered = format!("{} {} {}", cwd.display(), program, args.join(" "));
            self.commands.borrow_mut().push(rendered);
            if self.fail_program == Some(program)
                || self.fail_command_prefix.is_some_and(|prefix| {
                    format!("{program} {}", args.join(" ")).starts_with(prefix)
                })
            {
                return Err(format!("{program} failed in fixture"));
            }
            if program == "gh" {
                return Ok(PublishCommandOutput {
                    stdout: "https://github.com/traverse-framework/registry/pull/123".to_string(),
                });
            }
            Ok(PublishCommandOutput {
                stdout: String::new(),
            })
        }
    }

    fn capability_publish_fixture() -> CapabilityPublishFixture {
        let temp_dir = unique_temp_dir();
        let contract_path = temp_dir.join("contract.json");
        let artifact_path = temp_dir.join("artifact.wasm");
        let registry_repo_path = temp_dir.join("registry");
        fs::copy(
            repo_root()
                .join("contracts/examples/traverse-starter/capabilities/process/contract.json"),
            &contract_path,
        )
        .expect("capability contract fixture should copy");
        // Spec 102 FR-004: publish fixtures need a non-empty use_cases surface.
        let mut contract: Value = serde_json::from_str(
            &fs::read_to_string(&contract_path).expect("copied contract should read"),
        )
        .expect("copied contract should parse");
        contract["use_cases"] = serde_json::json!([
            {
                "scenario": "Process a starter note into structured metadata.",
                "input_example": { "note": "Ship the starter app path" },
                "output_example": {
                    "title": "Ship the starter app path",
                    "tags": ["starter"],
                    "noteType": "task",
                    "suggestedNextAction": "review",
                    "status": "ok"
                },
                "happy": true
            }
        ]);
        // Author evidence must survive normalize (validate_contract clears it).
        contract["evidence"] = serde_json::json!([
            {
                "evidence_id": "fixture-evd-1",
                "type": "contract_validation",
                "status": "passed"
            }
        ]);
        fs::write(
            &contract_path,
            serde_json::to_string_pretty(&contract).expect("fixture contract should serialize"),
        )
        .expect("fixture contract should write");
        fs::write(&artifact_path, b"fixture wasm bytes").expect("artifact fixture should write");
        fs::create_dir_all(&registry_repo_path).expect("registry fixture should create");

        CapabilityPublishFixture {
            contract: contract_path,
            artifact: artifact_path,
            registry_repo: registry_repo_path,
        }
    }

    #[derive(Clone)]
    struct StaticRegistryFetcher {
        result: Result<FetchedRegistryIndex, RegistrySyncError>,
    }

    impl RegistryIndexFetcher for StaticRegistryFetcher {
        fn fetch_latest_index(&self) -> Result<FetchedRegistryIndex, RegistrySyncError> {
            self.result.clone()
        }
    }

    fn registry_index_fixture() -> PublicRegistryIndex {
        PublicRegistryIndex {
            index_version: 7,
            generated_at: "2026-07-06T00:00:00Z".to_string(),
            source_commit: Some("abc123".to_string()),
            capabilities: vec![PublicRegistryCapabilityRecord {
                namespace: "traverse-starter".to_string(),
                id: "traverse-starter.process".to_string(),
                version: "1.0.0".to_string(),
                digest: "sha256:5647c39a".to_string(),
                artifact_url: "https://github.com/traverse-framework/registry/releases/download/artifacts/traverse-starter.process-1.0.0/traverse-starter.wasm".to_string(),
                contract_digest: "sha256:5647c39a".to_string(),
                contract_url: "https://github.com/traverse-framework/registry/releases/download/artifacts/traverse-starter.process-1.0.0/contract.json".to_string(),
                deprecated: false,
            }],
        }
    }

    fn registry_record_fixture(
        namespace: &str,
        id: &str,
        version: &str,
    ) -> PublicRegistryCapabilityRecord {
        PublicRegistryCapabilityRecord {
            namespace: namespace.to_string(),
            id: id.to_string(),
            version: version.to_string(),
            digest: "sha256:fixture".to_string(),
            artifact_url: "https://example.test/artifact.wasm".to_string(),
            contract_digest: "sha256:fixture-contract".to_string(),
            contract_url: "https://example.test/contract.json".to_string(),
            deprecated: false,
        }
    }

    fn write_registry_ref_app_fixture(
        temp_dir: &Path,
        artifact_digest: &str,
        version_range: &str,
    ) -> PathBuf {
        let manifest_path =
            write_app_validate_fixture(temp_dir, artifact_digest, artifact_digest, None);
        let component_path = temp_dir.join("component.manifest.json");
        let mut component: Value = serde_json::from_str(
            &fs::read_to_string(&component_path).expect("component manifest should read"),
        )
        .expect("component manifest should parse");
        let component_object = component
            .as_object_mut()
            .expect("component manifest should be an object");
        component_object.remove("contract_path");
        component_object.remove("wasm_binary_path");
        component_object.remove("wasm_digest");
        component_object.insert(
            "registry_ref".to_string(),
            serde_json::json!({
                "namespace": "fixture",
                "id": "expedition.planning.validate-team-readiness",
                "version_range": version_range
            }),
        );
        fs::write(
            component_path,
            serde_json::to_string_pretty(&component).expect("component manifest should serialize"),
        )
        .expect("component manifest should write");
        manifest_path
    }

    fn write_app_validate_fixture(
        temp_dir: &Path,
        app_digest: &str,
        component_digest: &str,
        workspace_config: Option<Value>,
    ) -> PathBuf {
        let repo = repo_root();
        let component_manifest_path =
            write_app_validate_component_fixture(temp_dir, &repo, component_digest);
        let mut workspace_defaults = serde_json::json!({ "workspace_id": "expedition-local" });
        if let Some(config) = workspace_config {
            let config_path = temp_dir.join("workspace.config.json");
            fs::write(
                &config_path,
                serde_json::to_string_pretty(&config).expect("workspace config must serialize"),
            )
            .expect("workspace config must write");
            workspace_defaults["config_path"] = Value::String("workspace.config.json".to_string());
        }

        let manifest_path = temp_dir.join("app.manifest.json");
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&serde_json::json!({
                "app_id": "expedition.readiness",
                "version": "1.0.0",
                "schema_version": "1.0.0",
                "workspace_defaults": workspace_defaults,
                "components": [{
                    "component_id": "expedition.readiness.validate-team-readiness-component",
                    "version": "1.0.0",
                    "digest": app_digest,
                    "manifest_path": component_manifest_path.display().to_string()
                }],
                "workflows": [{
                    "workflow_id": "expedition.planning.plan-expedition",
                    "workflow_version": "1.0.0",
                    "path": repo.join("workflows/examples/expedition/plan-expedition/workflow.json").display().to_string()
                }],
                "model_dependencies": [{
                    "interface_id": "traverse.inference.generate",
                    "version_range": "^1.0",
                    "selection_policy": {
                        "strategy": "priority",
                        "allow_fallback": true
                    },
                    "required_capabilities": ["text_generation"],
                    "minimum_context_window": 8192,
                    "candidates": [{
                        "candidate_id": "ollama-llama-3-2-readiness",
                        "provider_capability_id": "traverse.inference.generate",
                        "provider_implementation_id": "ollama.local.generate",
                        "model_identifier": "llama3.2:3b",
                        "placement_target": "local",
                        "priority": 10,
                        "required_provider_config_keys": ["ollama_base_url"],
                        "metadata": {
                            "provider": "ollama"
                        }
                    }]
                }],
                "config_schema": {
                    "type": "object",
                    "required": ["workspace_id"],
                    "properties": {
                        "workspace_id": {
                            "type": "string"
                        },
                        "readiness_mode": {
                            "type": "string",
                            "x-traverse-overrideable": true
                        }
                    },
                    "additionalProperties": false
                },
                "default_config": {
                    "workspace_id": "expedition-local",
                    "readiness_mode": "deterministic"
                },
                "placement_policy": {
                    "preferred_targets": ["local"],
                    "allow_fallback": false
                },
                "state_machine": app_state_machine_fixture(),
                "public_surfaces": ["cli"]
            }))
            .expect("app manifest must serialize"),
        )
        .expect("app manifest must write");
        manifest_path
    }

    fn add_universal_connector_bindings(manifest_path: &Path) {
        let mut manifest: Value = serde_json::from_str(
            &fs::read_to_string(manifest_path).expect("app manifest should read"),
        )
        .expect("app manifest should parse");
        manifest["connector_bindings"] = serde_json::json!([
            {
                "connector_id": "traverse.object-store",
                "version_range": "^1.0.0",
                "config_ref": "object-store.default"
            },
            {
                "connector_id": "traverse.state-store",
                "version_range": "^1.0.0",
                "config_ref": "state-store.default"
            },
            {
                "connector_id": "traverse.scheduler",
                "version_range": "^1.0.0",
                "config_ref": "scheduler.default"
            }
        ]);
        fs::write(
            manifest_path,
            serde_json::to_string_pretty(&manifest).expect("app manifest should serialize"),
        )
        .expect("app manifest should write");
    }

    fn universal_host_activation_json(connectors: &Value) -> String {
        serde_json::to_string_pretty(&serde_json::json!({
            "connectors": connectors,
            "artifacts": [{
                "contract_reference": "expedition.planning.validate-team-readiness@1.0.0",
                "placement_target": "local",
                "candidates": [{
                    "package_id": "fixture.team-readiness",
                    "package_version": "1.0.0",
                    "digest": "sha256:470e430bb7e53d2b4d37af50186511a1f7f9ae903bc4f1524755f2a97014ef90",
                    "abi": "wasi-preview1",
                    "lifecycle": "active",
                    "placement": ["local"],
                    "execution_constraints": "fixture"
                }]
            }]
        }))
        .expect("host activation fixture should serialize")
    }

    fn app_state_machine_fixture() -> Value {
        serde_json::json!({
            "initial_state": "idle",
            "states": [
                {
                    "id": "idle",
                    "transitions": [{ "on": "submit", "to": "processing" }]
                },
                {
                    "id": "processing",
                    "invoke": {
                        "capability_id": "expedition.planning.validate-team-readiness",
                        "input_from": "command.payload"
                    },
                    "transitions": [
                        { "on": "capability_succeeded", "to": "results" },
                        { "on": "capability_failed", "to": "error" }
                    ]
                },
                {
                    "id": "results",
                    "transitions": [{ "on": "reset", "to": "idle" }]
                },
                {
                    "id": "error",
                    "transitions": [
                        { "on": "retry", "to": "processing", "with_last_payload": true },
                        { "on": "reset", "to": "idle" }
                    ]
                }
            ]
        })
    }

    fn write_app_validate_component_fixture(
        temp_dir: &Path,
        repo: &Path,
        component_digest: &str,
    ) -> PathBuf {
        let component_manifest_path = temp_dir.join("component.manifest.json");
        fs::write(
            &component_manifest_path,
            serde_json::to_string_pretty(&serde_json::json!({
                "component_id": "expedition.readiness.validate-team-readiness-component",
                "version": "1.0.0",
                "schema_version": "1.0.0",
                "capability_id": "expedition.planning.validate-team-readiness",
                "capability_version": "1.0.0",
                "contract_path": repo.join("contracts/examples/expedition/capabilities/validate-team-readiness/contract.json").display().to_string(),
                "wasm_binary_path": repo.join("examples/capabilities/team-readiness-agent/artifacts/validate-team-readiness-agent.wasm").display().to_string(),
                "wasm_digest": component_digest,
                "runtime_constraints": {
                    "host_api_access": "none",
                    "network_access": "forbidden",
                    "filesystem_access": "none"
                },
                "permitted_targets": ["local"],
                "dependencies": [],
                "connector_requirements": [],
                "validation_evidence": []
            }))
            .expect("component manifest must serialize"),
        )
        .expect("component manifest must write");
        component_manifest_path
    }

    fn read_tree(root: &PathBuf) -> String {
        let mut contents = String::new();
        for entry in fs::read_dir(root).expect("directory should read") {
            let path = entry.expect("directory entry should read").path();
            if path.is_dir() {
                contents.push_str(&read_tree(&path));
            } else {
                contents
                    .push_str(&fs::read_to_string(&path).expect("generated text file should read"));
            }
        }
        contents
    }

    struct CapabilityPackageFixture {
        manifest_path: PathBuf,
    }

    fn build_real_capability_package_artifact(manifest_path: &Path) {
        let package_dir = manifest_path
            .parent()
            .expect("real capability package manifest must have a package directory");
        let status = ProcessCommand::new("bash")
            .arg(package_dir.join("build-fixture.sh"))
            .status()
            .expect("real capability package fixture builder should start");
        assert!(
            status.success(),
            "real capability package fixture builder should succeed"
        );
    }

    fn create_interpret_expedition_intent_capability_fixture() -> CapabilityPackageFixture {
        create_capability_package_fixture(&CapabilityPackageFixtureSpec {
            package_id: "expedition.planning.interpret-expedition-intent",
            capability_id: "expedition.planning.interpret-expedition-intent",
            binary_name: "interpret-expedition-intent-agent.wasm",
            summary: "Governed WASM capability example for expedition intent interpretation.",
            contract_path: "contracts/examples/expedition/capabilities/interpret-expedition-intent/contract.json",
            model_interface: "expedition-intent-interpretation-v1",
            model_purpose: "Interpret free-form expedition planning intent into governed route preferences and assumptions.",
            workflow_id: "expedition.planning.plan-expedition",
        })
    }

    fn create_validate_team_readiness_capability_fixture() -> CapabilityPackageFixture {
        create_capability_package_fixture(&CapabilityPackageFixtureSpec {
            package_id: "expedition.planning.validate-team-readiness",
            capability_id: "expedition.planning.validate-team-readiness",
            binary_name: "validate-team-readiness-agent.wasm",
            summary: "Governed WASM capability example for expedition readiness validation.",
            contract_path: "contracts/examples/expedition/capabilities/validate-team-readiness/contract.json",
            model_interface: "expedition-readiness-validation-v1",
            model_purpose: "Validate expedition team readiness against governed objective, conditions, and team profile context.",
            workflow_id: "expedition.planning.plan-expedition",
        })
    }

    fn create_hello_world_capability_fixture() -> CapabilityPackageFixture {
        create_capability_package_fixture(&CapabilityPackageFixtureSpec {
            package_id: "hello.world.say-hello-agent",
            capability_id: "hello.world.say-hello",
            binary_name: "say-hello-agent.wasm",
            summary: "Minimal governed hello-world agent package for Traverse onboarding.",
            contract_path: "contracts/examples/hello-world/capabilities/say-hello/contract.json",
            model_interface: "hello-world-greeting-v1",
            model_purpose: "Produce a simple deterministic greeting string for onboarding validation.",
            workflow_id: "hello.world.say-hello",
        })
    }

    struct CapabilityPackageFixtureSpec<'a> {
        package_id: &'a str,
        capability_id: &'a str,
        binary_name: &'a str,
        summary: &'a str,
        contract_path: &'a str,
        model_interface: &'a str,
        model_purpose: &'a str,
        workflow_id: &'a str,
    }

    fn create_capability_package_fixture(
        spec: &CapabilityPackageFixtureSpec<'_>,
    ) -> CapabilityPackageFixture {
        let temp_dir = unique_temp_dir();
        let package_dir = temp_dir.join("capability-package");
        let artifact_dir = package_dir.join("artifacts");
        let source_dir = package_dir.join("src");
        fs::create_dir_all(&artifact_dir).expect("artifact directory should create");
        fs::create_dir_all(&source_dir).expect("source directory should create");

        let wasm_bytes = hex_to_bytes(
            "0061736d0100000001040160000003020100070a01065f737461727400000a040102000b",
        );
        let binary_path = artifact_dir.join(spec.binary_name);
        fs::write(&binary_path, &wasm_bytes).expect("wasm binary should write");
        fs::write(
            source_dir.join("agent.rs"),
            format!(
                "pub fn run() -> &'static str {{ \"{}\" }}\n",
                spec.capability_id
            ),
        )
        .expect("source file should write");

        let repo_root = repo_root();
        let manifest_path = package_dir.join("manifest.json");
        let manifest = format!(
            r#"{{
  "kind": "capability_package",
  "schema_version": "1.0.0",
  "package_id": "{}",
  "version": "1.0.0",
  "summary": "{}",
  "capability_ref": {{
    "id": "{}",
    "version": "1.0.0",
    "contract_path": "{}"
  }},
  "known_compositions": [
    {{
      "workflow_id": "{}",
      "workflow_version": "1.0.0"
    }}
  ],
  "source": {{
    "path": "./src/agent.rs",
    "language": "rust",
    "entry": "run"
  }},
  "binary": {{
    "path": "./artifacts/{}",
    "format": "wasm",
    "expected_digest": "{}",
    "abi_version": "1.0.0"
  }},
  "constraints": {{
    "host_api_access": "none",
    "network_access": "forbidden",
    "filesystem_access": "none"
  }},
  "model_dependencies": [
    {{
      "interface": "{}",
      "purpose": "{}"
    }}
  ]
}}"#,
            spec.package_id,
            spec.summary,
            spec.capability_id,
            repo_root.join(spec.contract_path).display(),
            spec.workflow_id,
            spec.binary_name,
            fnv1a64(&wasm_bytes),
            spec.model_interface,
            spec.model_purpose
        );
        fs::write(&manifest_path, manifest).expect("manifest should write");

        CapabilityPackageFixture { manifest_path }
    }

    fn hex_to_bytes(value: &str) -> Vec<u8> {
        value
            .as_bytes()
            .chunks(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).expect("hex pair should be utf8");
                u8::from_str_radix(pair, 16).expect("hex pair should parse")
            })
            .collect()
    }

    // ------------------------------------------------------------------
    // Help-text and dispatch coverage (#638, pass 1)
    // ------------------------------------------------------------------

    fn parse_help(args: &[&str]) -> String {
        let mut full = vec!["traverse-cli".to_string()];
        full.extend(args.iter().map(ToString::to_string));
        parse_command(&full).expect_err("--help must never parse into a command")
    }

    #[test]
    fn global_help_prints_usage() {
        assert!(parse_help(&["--help"]).contains("usage: traverse-cli"));
        assert!(parse_help(&["help"]).contains("usage: traverse-cli"));
        assert!(parse_help(&["unknown-family", "--help"]).contains("usage: traverse-cli"));
    }

    #[test]
    fn subcommand_help_covers_every_family_and_subcommand() {
        let pairs: &[(&str, Option<&str>)] = &[
            ("bundle", Some("inspect")),
            ("bundle", Some("register")),
            ("bundle", None),
            ("app", Some("new")),
            ("app", Some("validate")),
            ("app", Some("register")),
            ("app", Some("activate")),
            ("app", None),
            ("registry", Some("sync")),
            ("registry", None),
            ("component", Some("new")),
            ("component", None),
            ("capability-package", Some("inspect")),
            ("capability-package", Some("execute")),
            ("capability-package", None),
            ("artifact", Some("verify")),
            ("artifact", None),
            ("wasm", Some("abi")),
            ("wasm", None),
            ("workflow", Some("register")),
            ("workflow", Some("list")),
            ("workflow", Some("inspect")),
            ("workflow", None),
            ("expedition", Some("execute")),
            ("expedition", None),
            ("capability", Some("new")),
            ("capability", Some("inspect")),
            ("capability", Some("discover")),
            ("capability", Some("publish")),
            ("capability", None),
            ("event", Some("inspect")),
            ("event", Some("validate-product")),
            ("event", None),
            ("trace", Some("inspect")),
            ("trace", None),
            ("serve", None),
            ("telemetry", Some("enable")),
            ("telemetry", Some("disable")),
            ("telemetry", None),
        ];
        for (family, sub) in pairs {
            let help = match sub {
                Some(sub) => parse_help(&[family, sub, "--help"]),
                None => parse_help(&[family, "--help"]),
            };
            assert!(
                help.contains(family),
                "help for {family} {sub:?} must mention {family}: {help}"
            );
            if let Some(sub) = sub {
                assert!(
                    help.contains(sub),
                    "help for {family} {sub:?} must mention {sub}: {help}"
                );
            }
        }
    }

    #[test]
    fn parse_command_covers_federation_and_discovery_forms() {
        let argv = |parts: &[&str]| -> Vec<String> {
            let mut full = vec!["traverse-cli".to_string()];
            full.extend(parts.iter().map(ToString::to_string));
            full
        };

        assert!(matches!(
            parse_command(&argv(&["federation", "peers", "manifest.json"])),
            Ok(Command::FederationPeers { .. })
        ));
        assert!(matches!(
            parse_command(&argv(&["federation", "sync", "manifest.json"])),
            Ok(Command::FederationSync { .. })
        ));
        assert!(matches!(
            parse_command(&argv(&["federation", "status", "manifest.json"])),
            Ok(Command::FederationStatus { .. })
        ));
        assert!(parse_command(&argv(&["federation", "unknown", "manifest.json"])).is_err());

        assert!(matches!(
            parse_command(&argv(&[
                "capability",
                "discover",
                "manifest.json",
                "--json"
            ])),
            Ok(Command::CapabilityDiscover {
                json_output: true,
                ..
            })
        ));
        assert!(matches!(
            parse_command(&argv(&[
                "capability",
                "inspect",
                "contracts/examples/expedition/capabilities/capture-expedition-objective/contract.json"
            ])),
            Ok(Command::CapabilityInspect { .. })
        ));
        assert!(parse_command(&argv(&["capability", "discover"])).is_err());
        assert!(parse_command(&argv(&["capability", "inspect"])).is_err());
    }

    #[test]
    fn run_command_fails_cleanly_on_missing_inputs() {
        let missing = unique_temp_dir().join("missing");
        let commands = vec![
            Command::BundleInspect {
                manifest_path: missing.clone(),
                json_output: false,
            },
            Command::BundleRegister {
                manifest_path: missing.clone(),
                json_output: false,
            },
            Command::AppValidate {
                manifest_path: missing.clone(),
                workspace_id: None,
                json_output: false,
            },
            Command::AppRegister {
                manifest_path: missing.clone(),
                workspace_id: "ws".to_string(),
                json_output: true,
            },
            Command::CapabilityPackageInspect {
                manifest_path: missing.clone(),
            },
            Command::CapabilityInspect {
                contract_path: missing.clone(),
            },
            Command::CapabilityPackageExecute {
                manifest_path: missing.clone(),
                request_path: missing.clone(),
            },
            Command::WasmAbiVerify {
                wasm_paths: vec![missing.clone()],
            },
            Command::ArtifactVerify {
                artifact_path: missing.clone(),
            },
            Command::ArtifactSign {
                artifact_path: missing.clone(),
            },
            Command::FederationPeers {
                manifest_path: missing.clone(),
            },
            Command::FederationSync {
                manifest_path: missing.clone(),
            },
            Command::FederationStatus {
                manifest_path: missing.clone(),
            },
            Command::ExpeditionExecute {
                request_path: missing.clone(),
                trace_output_path: None,
                json_output: false,
                validate_only: false,
            },
            Command::CapabilityDiscover {
                manifest_path: missing.clone(),
                json_output: false,
            },
            Command::Event {
                contract_path: missing.clone(),
            },
            Command::EventValidateProduct {
                descriptor_path: missing.clone(),
            },
            Command::TraceInspect {
                trace_path: missing.clone(),
            },
            Command::WorkflowRegister {
                workflow_path: missing.clone(),
                workspace_id: "ws".to_string(),
            },
        ];
        for command in commands {
            assert!(
                run_command(command).is_err(),
                "command with missing input must fail cleanly"
            );
        }

        let activation = run_command(Command::AppActivate {
            manifest_path: missing.clone(),
            workspace_id: "ws".to_string(),
            host_activation_path: missing.clone(),
            json_output: true,
        })
        .expect("activation failures should return structured JSON evidence");
        assert!(activation.contains("activation_failed"));

        let serve = Command::Serve {
            bind_address: "127.0.0.1:0".to_string(),
            auth_mode: None,
            allow_unauthenticated: false,
            allowed_origins: Vec::new(),
            render_mobile_qr: false,
            grpc_bind_address: None,
            grpc_tls_cert_path: None,
            grpc_tls_key_path: None,
        };
        assert!(matches!(run_command(serve), Err(CliError::UsageError(_))));
    }

    // ------------------------------------------------------------------
    // Command internals and publish/sync helper coverage (#638, pass 2)
    // ------------------------------------------------------------------

    #[test]
    fn bundle_inspect_and_register_render_json_summaries() {
        let manifest = canonical_expedition_bundle_path();
        let inspected =
            inspect_bundle(&manifest, true).expect("canonical bundle must inspect as JSON");
        let summary: Value =
            serde_json::from_str(&inspected).expect("inspect summary must be valid JSON");
        assert!(
            summary["capabilities"]
                .as_u64()
                .expect("capability count must be numeric")
                >= 1
        );
        assert!(summary["capability_ids"].is_array());

        let registered =
            register_bundle(&manifest, true).expect("canonical bundle must register as JSON");
        let summary: Value =
            serde_json::from_str(&registered).expect("register summary must be valid JSON");
        assert!(
            summary["registered_capabilities"]
                .as_u64()
                .expect("registered count must be numeric")
                >= 1
        );
    }

    #[test]
    fn capability_publish_guards_reject_unsafe_inputs() {
        let parse = reject_private_contract_scope("not-json")
            .expect_err("non-JSON contract text must be rejected");
        assert_eq!(parse.0, "capability_publish_contract_parse_failed");
        let private = reject_private_contract_scope(r#"{"scope":"private"}"#)
            .expect_err("private scope must be rejected");
        assert_eq!(private.0, "capability_publish_private_scope");
        reject_private_contract_scope(r#"{"scope":"public"}"#)
            .expect("public scope must be accepted");

        for unsafe_segment in ["", " ", ".", "..", "a/b", "a\\b"] {
            let err = validate_registry_path_segment(unsafe_segment, "capability id")
                .expect_err("unsafe registry path segment must be rejected");
            assert_eq!(err.0, "capability_publish_invalid_registry_path");
        }
        validate_registry_path_segment("content.comments", "capability id")
            .expect("safe registry path segment must be accepted");

        let missing = publish_file_sha256_digest(&unique_temp_dir().join("missing.wasm"))
            .expect_err("missing artifact must fail digest computation");
        assert_eq!(missing.0, "capability_publish_artifact_read_failed");

        let dir = unique_temp_dir();
        fs::create_dir_all(&dir).expect("temp dir must be creatable");
        let artifact = dir.join("artifact.wasm");
        fs::write(&artifact, b"wasm-bytes").expect("artifact must write");
        let digest = publish_file_sha256_digest(&artifact).expect("digest must compute");
        assert!(digest.starts_with("sha256:"), "{digest}");
    }

    #[test]
    fn registry_sync_failure_json_renders_stable_shape() {
        let rendered = registry_sync_failure_json("ws-test", "sync_failed", "boom")
            .expect("failure JSON must render");
        let value: Value =
            serde_json::from_str(&rendered).expect("failure JSON must be valid JSON");
        assert_eq!(value["status"], "failed");
        assert_eq!(value["workspace"], "ws-test");
        assert_eq!(value["errors"][0]["code"], "sync_failed");
        assert_eq!(value["errors"][0]["severity"], "error");
    }

    #[test]
    fn generated_app_bundle_registration_guards_incomplete_bundles() {
        let missing = register_generated_app_bundle(
            "app-x",
            "ws",
            &unique_temp_dir().join("missing-manifest.json"),
        )
        .expect_err("missing manifest must fail");
        assert!(matches!(missing, CliError::IoError(_)), "{missing:?}");

        let dir = unique_temp_dir();
        fs::create_dir_all(&dir).expect("temp dir must be creatable");
        let manifest = dir.join("manifest.json");
        fs::write(&manifest, br#"{"components": [], "workflows": ["w"]}"#)
            .expect("manifest must write");
        let incomplete = register_generated_app_bundle("app-x", "ws", &manifest)
            .expect_err("empty components must fail registration");
        assert!(
            incomplete.message().contains("incomplete"),
            "{incomplete:?}"
        );
    }

    #[test]
    fn curl_text_reports_fetch_failures() {
        let err = curl_text("http://127.0.0.1:1/unreachable", None)
            .expect_err("unreachable url must fail to fetch");
        assert_eq!(err.code, "registry_fetch_failed");
        assert!(err.message.contains("failed to fetch"), "{}", err.message);
    }

    #[test]
    fn real_publish_process_runner_captures_output_and_failures() {
        let cwd = unique_temp_dir();
        fs::create_dir_all(&cwd).expect("cwd must be creatable");
        let runner = RealPublishProcessRunner;

        let ok = runner
            .run(&cwd, "echo", &["publish-ok".to_string()])
            .expect("echo must succeed");
        assert_eq!(ok.stdout, "publish-ok");

        let status_failure = runner
            .run(&cwd, "false", &[])
            .expect_err("a failing command must surface its status");
        assert!(
            status_failure.contains("exited with status"),
            "{status_failure}"
        );

        let spawn_failure = runner
            .run(&cwd, "definitely-not-a-real-program-406", &[])
            .expect_err("a missing program must surface a spawn error");
        assert!(
            spawn_failure.contains("failed to execute"),
            "{spawn_failure}"
        );
    }

    #[test]
    fn dirty_registry_checkout_blocks_publishing() {
        struct DirtyStatusRunner;
        impl PublishProcessRunner for DirtyStatusRunner {
            fn run(
                &self,
                _cwd: &Path,
                _program: &str,
                _args: &[String],
            ) -> Result<PublishCommandOutput, String> {
                Ok(PublishCommandOutput {
                    stdout: " M contracts/contract.json".to_string(),
                })
            }
        }

        let err = ensure_clean_registry_checkout(Path::new("registry"), &DirtyStatusRunner)
            .expect_err("a dirty registry checkout must block publishing");
        assert!(err.contains("uncommitted changes"), "{err}");
    }
}
