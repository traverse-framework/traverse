mod agent_packages;
mod browser_adapter;
mod federation_operator;
mod http_api;
mod supply_chain;

use agent_packages::load_agent_package;
use browser_adapter::serve_local_browser_adapter;
use federation_operator::{
    render_federation_peers, render_federation_status, render_federation_sync,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::env;
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::Component;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use traverse_contracts::{
    EventContract, EventValidationContext, parse_event_contract, validate_event_contract,
};
use traverse_contracts::{ViolationRecord, reference_connector_contracts};
use traverse_registry::{
    ApplicationRegistrationRequest, ApplicationRegistry, ArtifactDigests, BinaryFormat,
    BinaryReference, CapabilityArtifactRecord, CapabilityRegistration, CapabilityRegistry,
    ComposabilityMetadata, CompositionKind, CompositionPattern, ConnectorRegistration,
    DiscoveryQuery, EventRegistration, EventRegistry, ImplementationKind, LookupScope,
    RegistryBundle, RegistryProvenance, RegistryScope, SourceKind, SourceReference,
    WorkflowDefinition, WorkflowReference, WorkflowRegistration, WorkflowRegistry,
    load_application_bundle_manifest, load_registry_bundle,
};
use traverse_runtime::executor::{SUPPORTED_HOST_ABI_VERSION, verify_wasm_host_abi_bytes};
use traverse_runtime::{
    LocalExecutionFailure, LocalExecutionFailureCode, LocalExecutor, Runtime,
    RuntimeExecutionOutcome, RuntimeRequest, RuntimeResultStatus, RuntimeTrace,
    parse_runtime_request,
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
        json_output: bool,
    },
    AppRegister {
        manifest_path: PathBuf,
        workspace_id: String,
        json_output: bool,
    },
    ComponentNew {
        component_id: String,
    },
    BrowserAdapterServe {
        bind_address: String,
    },
    AgentInspect {
        manifest_path: PathBuf,
    },
    AgentExecute {
        manifest_path: PathBuf,
        request_path: PathBuf,
    },
    WasmAbiVerify {
        wasm_paths: Vec<PathBuf>,
    },
    ArtifactVerify {
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
    Event {
        contract_path: PathBuf,
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
        allow_unauthenticated: bool,
        allowed_origins: Vec<String>,
    },
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
        Ok(Command::BrowserAdapterServe { bind_address }) => {
            if let Err(error) = serve_local_browser_adapter(&bind_address) {
                eprintln!("{error}");
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Ok(Command::Serve {
            bind_address,
            allow_unauthenticated,
            allowed_origins,
        }) => {
            if let Err(error) = run_serve(bind_address, allow_unauthenticated, allowed_origins) {
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
            json_output,
        } => app_validate(&manifest_path, json_output),
        Command::AppRegister {
            manifest_path,
            workspace_id,
            json_output,
        } => app_register(&manifest_path, &workspace_id, json_output),
        Command::ComponentNew { component_id } => component_new(&component_id),
        Command::BrowserAdapterServe { .. } | Command::Serve { .. } => {
            Err(CliError::UsageError(usage()))
        }
        Command::AgentInspect { manifest_path } => inspect_agent(&manifest_path),
        Command::AgentExecute {
            manifest_path,
            request_path,
        } => execute_agent(&manifest_path, &request_path),
        Command::WasmAbiVerify { wasm_paths } => verify_wasm_abi_imports(&wasm_paths),
        Command::ArtifactVerify { artifact_path } => verify_supply_chain_artifact(&artifact_path),
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
        Command::Event { contract_path } => inspect_event(&contract_path),
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
        (Some("browser-adapter"), Some("serve")) => parse_browser_adapter_command(args),
        (Some("serve"), _) => parse_serve_command(args),
        (Some("app"), Some("new")) => parse_app_new_command(args),
        (Some("app"), Some("validate")) => parse_app_validate_command(args),
        (Some("app"), Some("register")) => parse_app_register_command(args),
        (Some("component"), Some("new")) => parse_component_new_command(args),
        (Some("federation"), Some(_)) => parse_federation_command(args),
        (Some("agent"), Some("execute")) => parse_agent_execute_command(args),
        (Some("artifact"), Some("verify")) => parse_artifact_verify_command(args),
        (Some("wasm"), Some("abi")) => parse_wasm_abi_command(args),
        (Some("expedition"), Some("execute")) => parse_expedition_execute_command(args),
        (Some("capability"), Some("discover")) => parse_capability_discover_command(args),
        (Some("workflow"), Some(_)) => parse_workflow_command(args),
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
        (Some("app"), _) => help_app(),
        (Some("component"), Some("new")) => help_component_new(),
        (Some("component"), _) => help_component(),
        (Some("agent"), Some("inspect")) => help_agent_inspect(),
        (Some("agent"), Some("execute")) => help_agent_execute(),
        (Some("agent"), _) => help_agent(),
        (Some("artifact"), Some("verify")) => help_artifact_verify(),
        (Some("artifact"), _) => help_artifact(),
        (Some("wasm"), Some("abi")) => help_wasm_abi(),
        (Some("wasm"), _) => help_wasm(),
        (Some("workflow"), Some("register")) => help_workflow_register(),
        (Some("workflow"), Some("list")) => help_workflow_list(),
        (Some("workflow"), Some("inspect")) => help_workflow_inspect(),
        (Some("workflow"), _) => help_workflow(),
        (Some("expedition"), Some("execute")) => help_expedition_execute(),
        (Some("expedition"), _) => help_expedition(),
        (Some("capability"), Some("inspect")) => help_capability_inspect(),
        (Some("capability"), Some("discover")) => help_capability_discover(),
        (Some("capability"), _) => help_capability(),
        (Some("event"), Some("inspect")) => help_event_inspect(),
        (Some("event"), _) => help_event(),
        (Some("trace"), Some("inspect")) => help_trace_inspect(),
        (Some("trace"), _) => help_trace(),
        (Some("browser-adapter"), Some("serve")) => help_browser_adapter_serve(),
        (Some("browser-adapter"), _) => help_browser_adapter(),
        (Some("serve"), _) => help_serve(),
        _ => usage(),
    }
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
    "traverse-cli app validate --manifest <path> --json

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

fn help_app() -> String {
    "traverse-cli app <subcommand> [options]

  Subcommands:
    new <app-id>                 Create a governed Traverse app bundle scaffold.
    validate --manifest <path>   Validate an app bundle and emit JSON evidence.
    register --manifest <path>   Validate and persist local app registration.

  Run `traverse-cli app <subcommand> --help` for subcommand-specific help."
        .to_string()
}

fn help_component_new() -> String {
    "traverse-cli component new <component-id>

  Purpose:
    Create a governed WASM component package directory under
    components/<component-id>. The scaffold contains a schema-valid component
    manifest, draft capability contract, Rust package shell, source directory,
    and artifact directory. It does not create an executable WASM artifact.

  Required arguments:
    <component-id>   Component and capability id to scaffold.

  Optional flags:
    --help           Print this help text.

  Example:
    traverse-cli component new knowledge.retrieve"
        .to_string()
}

fn help_component() -> String {
    "traverse-cli component <subcommand> [options]

  Subcommands:
    new <component-id>   Create a governed WASM component package scaffold.

  Run `traverse-cli component new --help` for subcommand-specific help."
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

fn help_agent_inspect() -> String {
    "traverse-cli agent inspect <manifest-path>

  Purpose:
    Load and summarize a governed WASM agent package manifest. Verifies the
    binary digest, resolves the capability contract, and prints package metadata
    including model dependencies and workflow references.

  Required arguments:
    <manifest-path>   Path to the agent package manifest.json file.

  Optional flags:
    --help            Print this help text.

  Example:
    traverse-cli agent inspect examples/agents/expedition-intent-agent/manifest.json"
        .to_string()
}

fn help_agent_execute() -> String {
    "traverse-cli agent execute <manifest-path> <request-path>

  Purpose:
    Load a governed WASM agent package and execute it against a runtime request.
    Validates the package binary digest, registers the capability, and runs the
    request through the Traverse runtime.

  Required arguments:
    <manifest-path>   Path to the agent package manifest.json file.
    <request-path>    Path to the runtime request JSON file.

  Optional flags:
    --help            Print this help text.

  Example:
    traverse-cli agent execute \\
      examples/agents/expedition-intent-agent/manifest.json \\
      examples/agents/runtime-requests/interpret-expedition-intent.json"
        .to_string()
}

fn help_agent() -> String {
    "traverse-cli agent <subcommand> [options]

  Subcommands:
    inspect <manifest-path>                      Summarize a governed agent package.
    execute <manifest-path> <request-path>       Execute an agent against a runtime request.

  Run `traverse-cli agent <subcommand> --help` for subcommand-specific help."
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

fn help_artifact() -> String {
    "traverse-cli artifact <subcommand> [options]

  Subcommands:
    verify <artifact-or-manifest-path>   Verify checksum, signature, and provenance evidence.

  Run `traverse-cli artifact verify --help` for subcommand-specific help."
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

fn help_capability() -> String {
    "traverse-cli capability <subcommand> [options]

  Subcommands:
    inspect <contract-path>         Parse and validate a capability contract.
    discover <manifest-path>        List capabilities from a registry bundle.

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

fn help_event() -> String {
    "traverse-cli event <subcommand> [options]

  Subcommands:
    inspect <contract-path>   Parse and validate an event contract.

  Run `traverse-cli event inspect --help` for subcommand-specific help."
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

fn help_browser_adapter_serve() -> String {
    "traverse-cli browser-adapter serve [--bind <address>]

  Purpose:
    Start the local browser adapter proxy. The adapter bridges browser-side
    consumers to the local Traverse runtime over a same-origin HTTP endpoint.
    Stays running until stopped (Ctrl-C).

  Optional flags:
    --bind <address>   Address and port to listen on (default: 127.0.0.1:0).
    --help             Print this help text.

  Example:
    traverse-cli browser-adapter serve --bind 127.0.0.1:4174"
        .to_string()
}

fn help_browser_adapter() -> String {
    "traverse-cli browser-adapter <subcommand> [options]

  Subcommands:
    serve [--bind <address>]   Start the local browser adapter proxy.

  Run `traverse-cli browser-adapter serve --help` for subcommand-specific help."
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
    Ok(Command::AppValidate {
        manifest_path: PathBuf::from(manifest_path),
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

fn parse_component_new_command(args: &[String]) -> Result<Command, String> {
    match args {
        [_, _, _, component_id] => Ok(Command::ComponentNew {
            component_id: component_id.clone(),
        }),
        _ => Err(usage()),
    }
}

fn parse_browser_adapter_command(args: &[String]) -> Result<Command, String> {
    match args.len() {
        3 => Ok(Command::BrowserAdapterServe {
            bind_address: "127.0.0.1:0".to_string(),
        }),
        5 if args[3] == "--bind" => Ok(Command::BrowserAdapterServe {
            bind_address: args[4].clone(),
        }),
        _ => Err(usage()),
    }
}

fn help_serve() -> String {
    "traverse-cli serve [--bind <address>] [--port <port>] [--allow-unauthenticated]

  Purpose:
    Start a long-running HTTP/JSON API server on 127.0.0.1:8787 by default.
    Writes .traverse/server.json for local app discovery and exposes:
      GET  /healthz                    Returns the spec 033 health envelope.
      GET  /v1/capabilities            Returns JSON array of registered capabilities.
      POST /v1/capabilities/execute    Accepts RuntimeRequest JSON, returns trace + result.

    Loopback callers (127.0.0.1 / ::1) are allowed without authentication. All
    other callers must supply an Authorization: Bearer <token> header unless
    --allow-unauthenticated is set.

  Optional flags:
    --bind <address>           Address and port to listen on (default: 127.0.0.1:8787).
    --port <N>                 Compatibility shortcut for --bind 127.0.0.1:<N>.
    --allow-origin <origin>    Allow an exact browser Origin, repeatable for
                               production web apps. Wildcard '*' is rejected.
    --allow-unauthenticated    Accept unauthenticated requests from non-loopback
                               addresses. Prints a warning to stderr. Unsafe in
                               production.
    --help                     Print this help text.

  Example:
    traverse-cli serve
    traverse-cli serve --bind 127.0.0.1:9090
    traverse-cli serve --port 9090 --allow-unauthenticated"
        .to_string()
}

fn parse_serve_command(args: &[String]) -> Result<Command, String> {
    let allow_unauthenticated = args.iter().any(|a| a == "--allow-unauthenticated");
    let bind_flag_pos = args.iter().position(|a| a == "--bind");
    let port_flag_pos = args.iter().position(|a| a == "--port");
    let mut allowed_origins = Vec::new();

    if bind_flag_pos.is_some() && port_flag_pos.is_some() {
        return Err("--bind and --port cannot be used together".to_string());
    }

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
        format!("127.0.0.1:{port}")
    } else {
        "127.0.0.1:8787".to_string()
    };

    Ok(Command::Serve {
        bind_address,
        allow_unauthenticated,
        allowed_origins,
    })
}

fn run_serve(
    bind_address: String,
    allow_unauthenticated: bool,
    allowed_origins: Vec<String>,
) -> Result<(), String> {
    let registered =
        load_registered_bundle(&canonical_expedition_bundle_path()).map_err(|e| e.to_string())?;

    let config = http_api::ApiServerConfig {
        bind_address,
        allow_unauthenticated,
        allowed_origins,
        capability_registry: registered.capability_registry,
        workflow_registry: registered.workflow_registry,
        registry_root: std::env::current_dir()
            .map_err(|e| format!("failed to resolve current directory: {e}"))?
            .join(".traverse/registry"),
        executor: ExpeditionExampleExecutor,
        idempotency_retention_seconds: None,
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
        ("agent", "inspect") => Ok(Command::AgentInspect {
            manifest_path: PathBuf::from(positional[3]),
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

fn parse_agent_execute_command(args: &[String]) -> Result<Command, String> {
    match args {
        [_, _, _, manifest_path, request_path] => Ok(Command::AgentExecute {
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
    let registered = load_registered_bundle(manifest_path)?;
    if json_output {
        let json = serde_json::json!({
            "registered_capabilities": registered.capability_records.len(),
            "registered_events": registered.event_records.len(),
            "registered_workflows": registered.workflow_records.len(),
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
        ))
    }
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

fn component_new(component_id: &str) -> Result<String, CliError> {
    let base_dir = env::current_dir()
        .map_err(|e| CliError::IoError(format!("failed to resolve current directory: {e}")))?;
    component_new_at(&base_dir, component_id)
}

fn component_new_at(base_dir: &Path, component_id: &str) -> Result<String, CliError> {
    validate_scaffold_id(component_id, "component id")?;
    let component_dir = base_dir.join("components").join(component_id);
    if component_dir.exists() {
        return Err(CliError::IoError(format!(
            "component scaffold target already exists: {}",
            component_dir.display()
        )));
    }

    let src_dir = component_dir.join("src");
    let artifacts_dir = component_dir.join("artifacts");
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

    let component_name = scaffold_leaf_name(component_id);
    let wasm_name = format!("{component_name}.wasm");
    let package_name = component_id.replace('.', "-");
    write_new_file(
        &component_dir.join("Cargo.toml"),
        &format!(
            "[package]\nname = \"{package_name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[lib]\ncrate-type = [\"cdylib\"]\npath = \"src/lib.rs\"\n"
        ),
    )?;
    write_new_file(&src_dir.join("lib.rs"), "")?;
    write_new_file(
        &artifacts_dir.join("README.md"),
        "Place the compiled WASM artifact here after the component implementation is built.\n",
    )?;
    write_pretty_json(
        &component_dir.join("contract.json"),
        &component_contract_json(component_id, &component_name),
    )?;
    write_pretty_json(
        &component_dir.join("manifest.json"),
        &serde_json::json!({
            "component_id": component_id,
            "version": "1.0.0",
            "schema_version": "1.0.0",
            "capability_id": component_id,
            "capability_version": "1.0.0",
            "contract_path": "contract.json",
            "wasm_binary_path": format!("artifacts/{wasm_name}"),
            "wasm_digest": "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "runtime_constraints": {
                "host_api_access": "none",
                "network_access": "forbidden",
                "filesystem_access": "none"
            },
            "permitted_targets": ["local"],
            "dependencies": [],
            "connector_requirements": [],
            "validation_evidence": []
        }),
    )?;

    Ok([
        format!("created_component: {component_id}"),
        format!("component_dir: {}", component_dir.display()),
        format!(
            "manifest: {}",
            component_dir.join("manifest.json").display()
        ),
        format!(
            "contract: {}",
            component_dir.join("contract.json").display()
        ),
        format!("source: {}", src_dir.join("lib.rs").display()),
    ]
    .join("\n"))
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

fn app_validate(manifest_path: &Path, json_output: bool) -> Result<String, CliError> {
    if !json_output {
        return Err(CliError::UsageError(
            "app validate requires --json for stable setup evidence".to_string(),
        ));
    }

    if let Some(error) = validate_app_manifest_metadata_for_cli(manifest_path)? {
        return render_app_validation_failure(manifest_path, vec![error]);
    }

    match load_application_bundle_manifest(manifest_path) {
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

    let manifest = match load_application_bundle_manifest(manifest_path) {
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
                "capability_id": component.manifest.capability_id.clone(),
                "capability_version": component.manifest.capability_version.clone(),
                "wasm_digest": component.verified_wasm_digest.clone(),
                "manifest_path": component.manifest_path.display().to_string(),
                "contract_path": component.contract_path.display().to_string(),
                "artifact_ref": component.wasm_binary_path.display().to_string()
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
        .map(|component| {
            serde_json::json!({
                "component_id": component.manifest.component_id.clone(),
                "component_version": component.manifest.version.clone(),
                "path": component.wasm_binary_path.display().to_string(),
                "wasm_digest": component.verified_wasm_digest.clone(),
                "status": "verified"
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
        .map(|component| {
            serde_json::json!({
                "component_id": component.manifest.component_id.clone(),
                "component_version": component.manifest.version.clone(),
                "path": component.wasm_binary_path.display().to_string(),
                "wasm_digest": component.verified_wasm_digest.clone(),
                "status": "verified"
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
                "capability_id": component.manifest.capability_id.clone(),
                "capability_version": component.manifest.capability_version.clone(),
                "manifest_path": component.manifest_path.display().to_string(),
                "contract_path": component.contract_path.display().to_string(),
                "wasm_digest": component.verified_wasm_digest.clone()
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
    }

    Ok(None)
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

fn component_contract_json(component_id: &str, name: &str) -> Value {
    let namespace = component_namespace(component_id);
    serde_json::json!({
        "kind": "capability_contract",
        "schema_version": "1.0.0",
        "id": component_id,
        "namespace": namespace,
        "name": name,
        "version": "1.0.0",
        "lifecycle": "draft",
        "owner": {
            "team": "local-author",
            "contact": "local-author"
        },
        "summary": format!("Governed capability contract for {component_id}."),
        "description": format!("Draft Traverse capability contract for the real WASM component {component_id}."),
        "inputs": {
            "schema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {}
            }
        },
        "outputs": {
            "schema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {}
            }
        },
        "preconditions": [],
        "postconditions": [],
        "side_effects": [
            {
                "kind": "memory_only",
                "description": "No external side effects are declared for this draft component contract."
            }
        ],
        "emits": [],
        "consumes": [],
        "permissions": [
            {
                "id": component_id
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
            "created_at": "app-scaffold",
            "spec_ref": "044-application-bundle-manifest@1.0.0",
            "adr_refs": [],
            "exception_refs": []
        },
        "evidence": [],
        "service_type": "stateless",
        "permitted_targets": ["local"],
        "artifact_type": "native"
    })
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
                serde_json::json!({
                    "id": entry.id,
                    "version": entry.version,
                    "scope": format!("{:?}", entry.scope).to_lowercase(),
                    "lifecycle": format!("{:?}", entry.lifecycle).to_lowercase(),
                    "implementation_kind": format!("{:?}", entry.implementation_kind).to_lowercase(),
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

fn inspect_agent(manifest_path: &Path) -> Result<String, CliError> {
    let package = load_agent_package(manifest_path).map_err(CliError::IoError)?;
    Ok(package.render_summary())
}

fn execute_agent(manifest_path: &Path, request_path: &Path) -> Result<String, CliError> {
    let package = load_agent_package(manifest_path).map_err(CliError::IoError)?;
    let request = load_runtime_request(request_path)?;
    let mut registry = CapabilityRegistry::new();
    registry
        .register(package.capability_registration())
        .map_err(|f| CliError::RegistrationConflict(render_registry_failure(f)))?;
    let runtime = Runtime::new(registry, AgentPackageExampleExecutor);
    let outcome = runtime.execute(request);

    if outcome.result.status == RuntimeResultStatus::Error {
        return Err(CliError::ExecutionFailed(render_runtime_execution_failure(
            &outcome,
        )));
    }

    Ok(render_agent_execution_summary(
        &package.manifest.package_id,
        &package.manifest.capability_ref.id,
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

fn canonical_expedition_runtime_outcome() -> Result<RuntimeExecutionOutcome, CliError> {
    execute_expedition_outcome(&canonical_expedition_request_path())
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

#[allow(dead_code)]
fn inspect_workflow(workflow_path: &Path) -> Result<String, CliError> {
    let contents = read_text_file(workflow_path, "workflow artifact")?;
    let definition = serde_json::from_str::<WorkflowDefinition>(&contents).map_err(|error| {
        CliError::ValidationFailed(format!(
            "failed to parse workflow artifact {}: {error}",
            workflow_path.display()
        ))
    })?;

    Ok(render_workflow_summary(workflow_path, &definition))
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
    Ok(http_api::InProcessApi::new(http_api::ApiServerConfig {
        bind_address: "127.0.0.1:0".to_string(),
        allow_unauthenticated: true,
        allowed_origins: Vec::new(),
        capability_registry: registered.capability_registry,
        workflow_registry: registered.workflow_registry,
        registry_root: std::env::current_dir()
            .map_err(|e| CliError::IoError(format!("failed to resolve current directory: {e}")))?
            .join(".traverse/registry"),
        executor: ExpeditionExampleExecutor,
        idempotency_retention_seconds: None,
    }))
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

    lines.join("\n")
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

#[allow(dead_code)]
fn render_workflow_summary(path: &Path, definition: &WorkflowDefinition) -> String {
    let mut lines = vec![
        format!("path: {}", path.display()),
        format!("id: {}", definition.id),
        format!("version: {}", definition.version),
        format!("lifecycle: {:?}", definition.lifecycle).to_lowercase(),
        format!("start_node: {}", definition.start_node),
        format!("terminal_nodes: {}", definition.terminal_nodes.join(",")),
        format!("node_count: {}", definition.nodes.len()),
        format!("edge_count: {}", definition.edges.len()),
        format!("governing_spec: {}", definition.governing_spec),
        "node_capabilities:".to_string(),
    ];

    for node in &definition.nodes {
        lines.push(format!(
            "  - {} -> {}@{}",
            node.node_id, node.capability_id, node.capability_version
        ));
    }

    lines.push("edges:".to_string());
    for edge in &definition.edges {
        lines.push(format!(
            "  - {}: {} -> {}",
            edge.edge_id, edge.from, edge.to
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

fn render_agent_execution_summary(
    package_id: &str,
    capability_id: &str,
    outcome: &RuntimeExecutionOutcome,
) -> String {
    let output = outcome.result.output.as_ref().unwrap_or(&Value::Null);
    let mut lines = vec![
        format!("request_id: {}", outcome.result.request_id),
        format!("execution_id: {}", outcome.result.execution_id),
        format!("package_id: {package_id}"),
        format!("capability_id: {capability_id}"),
        "capability_version: 1.0.0".to_string(),
        "status: completed".to_string(),
        format!("trace_ref: {}", outcome.result.trace_ref),
    ];

    match capability_id {
        "expedition.planning.interpret-expedition-intent" => {
            if let Some(intent_id) = output.get("intent_id").and_then(Value::as_str) {
                lines.push(format!("intent_id: {intent_id}"));
            }
            if let Some(objective_id) = output.get("objective_id").and_then(Value::as_str) {
                lines.push(format!("objective_id: {objective_id}"));
            }
            if let Some(confidence) = output.get("confidence").and_then(Value::as_f64) {
                lines.push(format!("confidence: {confidence:.2}"));
            }
            if let Some(route_preferences) =
                output.get("route_preferences").and_then(Value::as_array)
            {
                let joined = route_preferences
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ");
                lines.push(format!("route_preferences: {joined}"));
            }
        }
        "expedition.planning.validate-team-readiness" => {
            if let Some(readiness_result_id) =
                output.get("readiness_result_id").and_then(Value::as_str)
            {
                lines.push(format!("readiness_result_id: {readiness_result_id}"));
            }
            if let Some(objective_id) = output.get("objective_id").and_then(Value::as_str) {
                lines.push(format!("objective_id: {objective_id}"));
            }
            if let Some(status) = output.get("status").and_then(Value::as_str) {
                lines.push(format!("readiness_status: {status}"));
            }
            if let Some(required_actions) = output.get("required_actions").and_then(Value::as_array)
            {
                let joined = required_actions
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ");
                lines.push(format!("required_actions: {joined}"));
            }
        }
        "hello.world.say-hello" => {
            if let Some(name) = output.get("name").and_then(Value::as_str) {
                lines.push(format!("name: {name}"));
            }
            if let Some(greeting) = output.get("greeting").and_then(Value::as_str) {
                lines.push(format!("greeting: {greeting}"));
            }
        }
        _ => {}
    }

    lines.join("\n")
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
    "usage: traverse-cli app <new|validate|register> [options] | traverse-cli component new <component-id> | traverse-cli <bundle|agent|event|trace|workflow|expedition|federation> <inspect|register|execute|peers|sync|status> <artifact-path> [request-path] [--trace-out <trace-path>] | traverse-cli browser-adapter serve [--bind <address>] | traverse-cli serve [--bind <address>] [--port <N>] [--allow-unauthenticated]".to_string()
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
}

#[derive(Debug, Default, Clone, Copy)]
struct ExpeditionExampleExecutor;

impl LocalExecutor for ExpeditionExampleExecutor {
    fn execute(
        &self,
        capability: &traverse_registry::ResolvedCapability,
        input: &Value,
    ) -> Result<Value, LocalExecutionFailure> {
        match capability.contract.id.as_str() {
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
            "expedition.planning.assemble-expedition-plan" => {
                execute_assemble_expedition_plan(input)
            }
            other => Err(executor_failure(&format!(
                "unsupported expedition example capability: {other}"
            ))),
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct AgentPackageExampleExecutor;

impl LocalExecutor for AgentPackageExampleExecutor {
    fn execute(
        &self,
        capability: &traverse_registry::ResolvedCapability,
        input: &Value,
    ) -> Result<Value, LocalExecutionFailure> {
        match capability.contract.id.as_str() {
            "hello.world.say-hello" => execute_hello_world(input),
            "expedition.planning.interpret-expedition-intent" => {
                execute_interpret_expedition_intent(input)
            }
            "expedition.planning.validate-team-readiness" => execute_validate_team_readiness(input),
            other => Err(executor_failure(&format!(
                "unsupported AI agent capability: {other}"
            ))),
        }
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
    let bundle = load_registry_bundle(manifest_path).map_err(|failure| {
        let msg = failure.errors[0].message.clone();
        CliError::IoError(msg)
    })?;

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
    })
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

fn canonical_expedition_request_path() -> PathBuf {
    repo_root().join("examples/expedition/runtime-requests/plan-expedition.json")
}

fn execute_expedition_outcome(request_path: &Path) -> Result<RuntimeExecutionOutcome, CliError> {
    let request = load_runtime_request(request_path)?;
    let registered = load_registered_bundle(&canonical_expedition_bundle_path())?;
    let runtime = Runtime::new(registered.capability_registry, ExpeditionExampleExecutor)
        .with_workflow_registry(registered.workflow_registry);
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

fn execute_hello_world(input: &Value) -> Result<Value, LocalExecutionFailure> {
    let map = input_object(input)?;
    let name = required_string(map, "name")?;

    Ok(serde_json::json!({
        "name": name,
        "greeting": format!("Hello, {name}!"),
    }))
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
        Command, app_new_at, app_register_at, app_registration_state_path, app_validate,
        component_new_at, execute_agent, execute_expedition, inspect_agent, inspect_bundle,
        inspect_event, inspect_trace, inspect_workflow, parse_command, register_bundle,
    };
    use crate::agent_packages::fnv1a64;
    use serde_json::Value;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};
    use traverse_contracts::parse_contract;
    use traverse_registry::load_application_bundle_manifest;

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
        let agent_inspect = vec![
            "traverse-cli".to_string(),
            "agent".to_string(),
            "inspect".to_string(),
            "examples/agents/expedition-intent-agent/manifest.json".to_string(),
        ];
        let agent_execute = vec![
            "traverse-cli".to_string(),
            "agent".to_string(),
            "execute".to_string(),
            "examples/agents/expedition-intent-agent/manifest.json".to_string(),
            "examples/agents/runtime-requests/interpret-expedition-intent.json".to_string(),
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
        assert!(parse_command(&agent_inspect).is_ok());
        assert!(parse_command(&agent_execute).is_ok());
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
    fn app_validate_returns_validated_json_for_checked_in_app_manifest() {
        let manifest_path =
            repo_root().join("examples/applications/expedition-readiness/app.manifest.json");

        let output = app_validate(&manifest_path, true).expect("app validation should succeed");
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
            app_validate(&manifest_path, true).expect("validation failure is JSON evidence");
        let json: Value = serde_json::from_str(&output).expect("failure output must be JSON");

        assert_eq!(json["status"], "failed");
        assert_eq!(json["errors"][0]["code"], "placeholder_wasm_digest");
        assert_eq!(json["errors"][0]["severity"], "error");
    }

    #[test]
    fn app_validate_redacts_workspace_secret_keys() {
        let temp_dir = unique_temp_dir();
        let manifest_path = write_app_validate_fixture(
            &temp_dir,
            "sha256:5647c39a1d25d8728350f9619025292a62e78a602068a2ad9b6f075751c93d99",
            "sha256:5647c39a1d25d8728350f9619025292a62e78a602068a2ad9b6f075751c93d99",
            Some(serde_json::json!({
                "overrides": {
                    "readiness_mode": "deterministic"
                },
                "secrets": {
                    "ollama_api_key": "do-not-render"
                }
            })),
        );

        let output = app_validate(&manifest_path, true).expect("app validation should succeed");
        let json: Value = serde_json::from_str(&output).expect("validation output must be JSON");

        assert_eq!(json["status"], "validated");
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
            "sha256:5647c39a1d25d8728350f9619025292a62e78a602068a2ad9b6f075751c93d99",
            "sha256:5647c39a1d25d8728350f9619025292a62e78a602068a2ad9b6f075751c93d99",
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
    fn component_new_generates_manifest_contract_and_non_executable_package() {
        let temp_dir = unique_temp_dir();

        let output = component_new_at(&temp_dir, "knowledge.retrieve")
            .expect("component scaffold should be created");

        let component_dir = temp_dir.join("components/knowledge.retrieve");
        assert!(output.contains("created_component: knowledge.retrieve"));
        assert!(component_dir.join("manifest.json").is_file());
        assert!(component_dir.join("contract.json").is_file());
        assert!(component_dir.join("Cargo.toml").is_file());
        assert!(component_dir.join("src/lib.rs").is_file());
        assert!(!component_dir.join("artifacts/retrieve.wasm").exists());

        let manifest = serde_json::from_str::<serde_json::Value>(
            &fs::read_to_string(component_dir.join("manifest.json"))
                .expect("component manifest should read"),
        )
        .expect("component manifest should parse as JSON");
        assert_eq!(manifest["component_id"], "knowledge.retrieve");
        assert_eq!(manifest["capability_id"], "knowledge.retrieve");
        assert_eq!(manifest["wasm_binary_path"], "artifacts/retrieve.wasm");

        let contract_contents =
            fs::read_to_string(component_dir.join("contract.json")).expect("contract should read");
        let contract = parse_contract(&contract_contents).expect("contract should parse");
        assert_eq!(contract.id, "knowledge.retrieve");
        assert_eq!(contract.lifecycle, traverse_contracts::Lifecycle::Draft);
        assert!(!read_tree(&component_dir).contains("TODO"));
    }

    #[test]
    fn parse_serve_defaults_to_loopback_8787() {
        let args = vec!["traverse-cli".to_string(), "serve".to_string()];

        let command = parse_command(&args).expect("serve command should parse");

        match command {
            Command::Serve {
                bind_address,
                allow_unauthenticated,
                allowed_origins,
            } => {
                assert_eq!(bind_address, "127.0.0.1:8787");
                assert!(!allow_unauthenticated);
                assert!(allowed_origins.is_empty());
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
                allow_unauthenticated,
                ..
            } => {
                assert_eq!(bind_address, "127.0.0.1:9090");
                assert!(allow_unauthenticated);
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
    fn parse_command_returns_agent_inspect_help_on_help_flag() {
        let args = vec![
            "traverse-cli".to_string(),
            "agent".to_string(),
            "inspect".to_string(),
            "--help".to_string(),
        ];
        let result = parse_command(&args);
        assert!(result.is_err(), "expected Err for --help");
        let text = result.err().unwrap_or_default();
        assert!(text.contains("agent inspect"));
        assert!(text.contains("<manifest-path>"));
        assert!(text.contains("Example:"));
    }

    #[test]
    fn parse_command_returns_agent_execute_help_on_help_flag() {
        let args = vec![
            "traverse-cli".to_string(),
            "agent".to_string(),
            "execute".to_string(),
            "--help".to_string(),
        ];
        let result = parse_command(&args);
        assert!(result.is_err(), "expected Err for --help");
        let text = result.err().unwrap_or_default();
        assert!(text.contains("agent execute"));
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
    fn parse_command_returns_browser_adapter_serve_help_on_help_flag() {
        let args = vec![
            "traverse-cli".to_string(),
            "browser-adapter".to_string(),
            "serve".to_string(),
            "--help".to_string(),
        ];
        let result = parse_command(&args);
        assert!(result.is_err(), "expected Err for --help");
        let text = result.err().unwrap_or_default();
        assert!(text.contains("browser-adapter serve"));
        assert!(text.contains("--bind"));
        assert!(text.contains("Example:"));
    }

    #[test]
    fn parse_command_returns_family_help_when_only_family_and_help_flag() {
        let cases = vec![
            (vec!["traverse-cli", "bundle", "--help"], "bundle"),
            (vec!["traverse-cli", "agent", "--help"], "agent"),
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

        let output =
            register_bundle(&manifest_path, false).expect("bundle register should succeed");

        assert!(output.contains("registered_capabilities: 6"));
        assert!(output.contains("registered_events: 5"));
        assert!(output.contains("registered_workflows: 1"));
        assert!(output.contains("expedition.planning.plan-expedition@1.0.0 (workflow)"));
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
  "scope": "public",
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
    fn inspect_agent_renders_governed_wasm_agent_package() {
        let fixture = create_interpret_expedition_intent_agent_fixture();

        let output = inspect_agent(&fixture.manifest_path).expect("agent inspect should succeed");

        assert!(
            output.contains("package_id: expedition.planning.interpret-expedition-intent-agent")
        );
        assert!(output.contains("capability_id: expedition.planning.interpret-expedition-intent"));
        assert!(output.contains("binary_digest: fnv1a64:"));
        assert!(output.contains("workflow_refs: expedition.planning.plan-expedition@1.0.0"));
    }

    #[test]
    fn execute_agent_runs_governed_ai_agent_request() {
        let fixture = create_interpret_expedition_intent_agent_fixture();
        let request_path =
            repo_root().join("examples/agents/runtime-requests/interpret-expedition-intent.json");

        let output = execute_agent(&fixture.manifest_path, &request_path)
            .expect("agent execution should succeed");

        assert!(
            output.contains("package_id: expedition.planning.interpret-expedition-intent-agent")
        );
        assert!(output.contains("capability_id: expedition.planning.interpret-expedition-intent"));
        assert!(output.contains("status: completed"));
        assert!(output.contains("route_preferences: conservative-alpine-push, same-day-return"));
    }

    #[test]
    fn inspect_agent_renders_second_governed_wasm_agent_package() {
        let fixture = create_validate_team_readiness_agent_fixture();

        let output = inspect_agent(&fixture.manifest_path).expect("agent inspect should succeed");

        assert!(output.contains("package_id: expedition.planning.validate-team-readiness-agent"));
        assert!(output.contains("capability_id: expedition.planning.validate-team-readiness"));
        assert!(output.contains("binary_digest: fnv1a64:"));
        assert!(output.contains("workflow_refs: expedition.planning.plan-expedition@1.0.0"));
    }

    #[test]
    fn execute_agent_runs_second_governed_ai_agent_request() {
        let fixture = create_validate_team_readiness_agent_fixture();
        let request_path =
            repo_root().join("examples/agents/runtime-requests/validate-team-readiness.json");

        let output = execute_agent(&fixture.manifest_path, &request_path)
            .expect("agent execution should succeed");

        assert!(output.contains("package_id: expedition.planning.validate-team-readiness-agent"));
        assert!(output.contains("capability_id: expedition.planning.validate-team-readiness"));
        assert!(output.contains("status: completed"));
        assert!(output.contains("readiness_status: ready"));
    }

    #[test]
    fn inspect_agent_renders_hello_world_package() {
        let fixture = create_hello_world_agent_fixture();

        let output = inspect_agent(&fixture.manifest_path).expect("agent inspect should succeed");

        assert!(output.contains("package_id: hello.world.say-hello-agent"));
        assert!(output.contains("capability_id: hello.world.say-hello"));
        assert!(output.contains("binary_digest: fnv1a64:"));
        assert!(output.contains("workflow_refs: hello.world.say-hello@1.0.0"));
    }

    #[test]
    fn execute_agent_runs_hello_world_request() {
        let fixture = create_hello_world_agent_fixture();
        let request_path = repo_root().join("examples/hello-world/runtime-requests/say-hello.json");

        let output = execute_agent(&fixture.manifest_path, &request_path)
            .expect("hello-world agent execution should succeed");

        assert!(output.contains("package_id: hello.world.say-hello-agent"));
        assert!(output.contains("capability_id: hello.world.say-hello"));
        assert!(output.contains("status: completed"));
        assert!(output.contains("name: Traverse"));
        assert!(output.contains("greeting: Hello, Traverse!"));
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
    "scope": "public_only",
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
    fn inspect_workflow_renders_canonical_workflow() {
        let path = repo_root().join("workflows/examples/expedition/plan-expedition/workflow.json");

        let output = inspect_workflow(&path).expect("workflow inspect should succeed");

        assert!(output.contains("id: expedition.planning.plan-expedition"));
        assert!(output.contains("start_node: capture_objective"));
        assert!(output.contains("node_capabilities:"));
    }

    #[test]
    fn inspect_workflow_rejects_malformed_definition() {
        let temp_dir = unique_temp_dir();
        let path = temp_dir.join("workflow.json");
        fs::write(&path, "{\"id\":true}").expect("workflow file should write");

        let error = inspect_workflow(&path).expect_err("malformed workflow should fail");

        assert!(
            error
                .message()
                .contains("failed to parse workflow artifact")
        );
    }

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("traverse-cli-test-{nanos}"));
        fs::create_dir_all(&path).expect("temporary directory should create");
        path
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
                "public_surfaces": ["cli"]
            }))
            .expect("app manifest must serialize"),
        )
        .expect("app manifest must write");
        manifest_path
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
                "wasm_binary_path": repo.join("examples/agents/team-readiness-agent/artifacts/validate-team-readiness-agent.wasm").display().to_string(),
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

    struct AgentFixture {
        manifest_path: PathBuf,
    }

    fn create_interpret_expedition_intent_agent_fixture() -> AgentFixture {
        create_agent_package_fixture(&AgentPackageFixtureSpec {
            package_id: "expedition.planning.interpret-expedition-intent-agent",
            capability_id: "expedition.planning.interpret-expedition-intent",
            binary_name: "interpret-expedition-intent-agent.wasm",
            summary: "Governed WASM AI agent example for expedition intent interpretation.",
            contract_path: "contracts/examples/expedition/capabilities/interpret-expedition-intent/contract.json",
            model_interface: "expedition-intent-interpretation-v1",
            model_purpose: "Interpret free-form expedition planning intent into governed route preferences and assumptions.",
            workflow_id: "expedition.planning.plan-expedition",
        })
    }

    fn create_validate_team_readiness_agent_fixture() -> AgentFixture {
        create_agent_package_fixture(&AgentPackageFixtureSpec {
            package_id: "expedition.planning.validate-team-readiness-agent",
            capability_id: "expedition.planning.validate-team-readiness",
            binary_name: "validate-team-readiness-agent.wasm",
            summary: "Governed WASM AI agent example for expedition readiness validation.",
            contract_path: "contracts/examples/expedition/capabilities/validate-team-readiness/contract.json",
            model_interface: "expedition-readiness-validation-v1",
            model_purpose: "Validate expedition team readiness against governed objective, conditions, and team profile context.",
            workflow_id: "expedition.planning.plan-expedition",
        })
    }

    fn create_hello_world_agent_fixture() -> AgentFixture {
        create_agent_package_fixture(&AgentPackageFixtureSpec {
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

    struct AgentPackageFixtureSpec<'a> {
        package_id: &'a str,
        capability_id: &'a str,
        binary_name: &'a str,
        summary: &'a str,
        contract_path: &'a str,
        model_interface: &'a str,
        model_purpose: &'a str,
        workflow_id: &'a str,
    }

    fn create_agent_package_fixture(spec: &AgentPackageFixtureSpec<'_>) -> AgentFixture {
        let temp_dir = unique_temp_dir();
        let package_dir = temp_dir.join("agent");
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
  "kind": "agent_package",
  "schema_version": "1.0.0",
  "package_id": "{}",
  "version": "1.0.0",
  "summary": "{}",
  "capability_ref": {{
    "id": "{}",
    "version": "1.0.0",
    "contract_path": "{}"
  }},
  "workflow_refs": [
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

        AgentFixture { manifest_path }
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
}
