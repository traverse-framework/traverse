mod agent_packages;
mod browser_adapter;
mod federation_operator;
mod http_api;

use agent_packages::load_agent_package;
use browser_adapter::serve_local_browser_adapter;
use federation_operator::{
    render_federation_peers, render_federation_status, render_federation_sync,
};
use serde_json::Value;
use std::env;
use std::fs;
use std::path::Component;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use traverse_contracts::ViolationRecord;
use traverse_contracts::{
    EventContract, EventValidationContext, parse_event_contract, validate_event_contract,
};
use traverse_registry::{
    ArtifactDigests, BinaryFormat, BinaryReference, CapabilityArtifactRecord,
    CapabilityRegistration, CapabilityRegistry, ComposabilityMetadata, CompositionKind,
    CompositionPattern, DiscoveryQuery, EventRegistration, EventRegistry, ImplementationKind,
    LookupScope, RegistryBundle, RegistryProvenance, SourceKind, SourceReference,
    WorkflowDefinition, WorkflowReference, WorkflowRegistration, WorkflowRegistry,
    load_registry_bundle,
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
        Command::BrowserAdapterServe { .. } | Command::Serve { .. } => {
            Err(CliError::UsageError(usage()))
        }
        Command::AgentInspect { manifest_path } => inspect_agent(&manifest_path),
        Command::AgentExecute {
            manifest_path,
            request_path,
        } => execute_agent(&manifest_path, &request_path),
        Command::WasmAbiVerify { wasm_paths } => verify_wasm_abi_imports(&wasm_paths),
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
        (Some("federation"), Some(_)) => parse_federation_command(args),
        (Some("agent"), Some("execute")) => parse_agent_execute_command(args),
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
        (Some("agent"), Some("inspect")) => help_agent_inspect(),
        (Some("agent"), Some("execute")) => help_agent_execute(),
        (Some("agent"), _) => help_agent(),
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
    "usage: traverse-cli <bundle|agent|event|trace|workflow|expedition|federation> <inspect|register|execute|peers|sync|status> <artifact-path> [request-path] [--trace-out <trace-path>] | traverse-cli browser-adapter serve [--bind <address>] | traverse-cli serve [--bind <address>] [--port <N>] [--allow-unauthenticated]".to_string()
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
        Command, execute_agent, execute_expedition, inspect_agent, inspect_bundle, inspect_event,
        inspect_trace, inspect_workflow, parse_command, register_bundle,
    };
    use crate::agent_packages::fnv1a64;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

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

        assert!(parse_command(&bundle).is_ok());
        assert!(parse_command(&bundle_register).is_ok());
        assert!(parse_command(&agent_inspect).is_ok());
        assert!(parse_command(&agent_execute).is_ok());
        assert!(parse_command(&wasm_abi_verify).is_ok());
        assert!(parse_command(&expedition_execute).is_ok());
        assert!(parse_command(&expedition_execute_with_trace).is_ok());
        assert!(parse_command(&event).is_ok());
        assert!(parse_command(&trace).is_ok());
        assert!(parse_command(&workflow).is_ok());
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
