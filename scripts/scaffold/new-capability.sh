#!/usr/bin/env bash
set -euo pipefail

NAME=""
NAMESPACE=""
OUTPUT_DIR=""

usage() {
  echo "Usage: $0 --name <name> --namespace <namespace> [--output-dir <dir>]" >&2
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --name)
      NAME="$2"
      shift 2
      ;;
    --namespace)
      NAMESPACE="$2"
      shift 2
      ;;
    --output-dir)
      OUTPUT_DIR="$2"
      shift 2
      ;;
    *)
      usage
      ;;
  esac
done

[[ -z "$NAME" ]] && { echo "Error: --name is required" >&2; usage; }
[[ -z "$NAMESPACE" ]] && { echo "Error: --namespace is required" >&2; usage; }

if ! [[ "$NAME" =~ ^[a-z0-9-]+$ ]]; then
  echo "Error: --name must be kebab-case ([a-z0-9-] only), got: $NAME" >&2
  exit 1
fi

if ! [[ "$NAMESPACE" =~ ^[a-z0-9][a-z0-9.-]*[a-z0-9]$|^[a-z0-9]$ ]]; then
  echo "Error: --namespace must be dot-separated identifiers ([a-z0-9.-] only), got: $NAMESPACE" >&2
  exit 1
fi

if [[ -z "$OUTPUT_DIR" ]]; then
  OUTPUT_DIR="./scaffold/${NAMESPACE}/${NAME}"
fi

mkdir -p "$OUTPUT_DIR/src"

# contract.json
cat > "$OUTPUT_DIR/contract.json" <<EOF
{
  "kind": "capability_contract",
  "schema_version": "1.0.0",
  "id": "${NAMESPACE}.${NAME}",
  "namespace": "${NAMESPACE}",
  "name": "${NAME}",
  "version": "0.1.0",
  "lifecycle": "draft",
  "service_type": "stateless",
  "artifact_type": "native",
  "description": "TODO: describe what this capability does.",
  "input_schema": {
    "type": "object",
    "required": ["input"],
    "properties": {
      "input": { "type": "string", "description": "TODO: replace with your input fields" }
    },
    "additionalProperties": false
  },
  "output_schema": {
    "type": "object",
    "required": ["output"],
    "properties": {
      "output": { "type": "string", "description": "TODO: replace with your output fields" }
    },
    "additionalProperties": false
  },
  "execution": {
    "binary_format": "wasm",
    "entrypoint": { "kind": "wasi-command", "command": "run" },
    "preferred_targets": ["local"],
    "constraints": {
      "host_api_access": "none",
      "network_access": "forbidden",
      "filesystem_access": "none"
    }
  },
  "provenance": {
    "spec_refs": ["002-capability-contracts"],
    "exception_refs": []
  }
}
EOF

# Cargo.toml
cat > "$OUTPUT_DIR/Cargo.toml" <<EOF
[package]
name = "${NAME}"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "${NAME}"
path = "src/main.rs"

[dependencies]
serde_json = "1"
EOF

# src/main.rs
cat > "$OUTPUT_DIR/src/main.rs" <<'RUST_EOF'
use std::io::{self, Read, Write};

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap_or_default();

    let request: serde_json::Value = serde_json::from_str(&input)
        .unwrap_or(serde_json::Value::Null);

    // TODO: replace this stub with your logic
    let input_value = request
        .get("input")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let output = serde_json::json!({ "output": format!("processed: {}", input_value) });

    let _ = io::stdout().write_all(output.to_string().as_bytes());
}
RUST_EOF

# build-fixture.sh
cat > "$OUTPUT_DIR/build-fixture.sh" <<EOF
#!/usr/bin/env bash
set -euo pipefail

AGENT_DIR="\$(cd "\$(dirname "\$0")" && pwd)"
NAME="${NAME}"

cargo build \\
  --manifest-path "\$AGENT_DIR/Cargo.toml" \\
  --target wasm32-wasip1 \\
  --release 2>&1

mkdir -p "\$AGENT_DIR/fixture"
cp "target/wasm32-wasip1/release/\${NAME}.wasm" "\$AGENT_DIR/fixture/agent.wasm"

DIGEST=\$(shasum -a 256 "\$AGENT_DIR/fixture/agent.wasm" | awk '{print \$1}')
echo "Built: \$AGENT_DIR/fixture/agent.wasm"
echo "SHA-256: \$DIGEST"
echo ""
echo "Update contract.json or manifest.json binary.digest with: \$DIGEST"
EOF
chmod +x "$OUTPUT_DIR/build-fixture.sh"

# runtime-request.json
cat > "$OUTPUT_DIR/runtime-request.json" <<EOF
{
  "kind": "runtime_request",
  "schema_version": "1.0.0",
  "request_id": "${NAME}-test-001",
  "intent": {
    "capability_id": "${NAMESPACE}.${NAME}",
    "capability_version": "0.1.0"
  },
  "input": { "input": "hello" },
  "lookup": { "scope": "public_only", "allow_ambiguity": false },
  "context": { "requested_target": "local", "caller": "scaffold-test" },
  "governing_spec": "006-runtime-request-execution"
}
EOF

echo ""
echo "Scaffold created at: ${OUTPUT_DIR}"
echo ""
echo "Files generated:"
echo "  contract.json         — capability contract (edit input_schema, output_schema, description)"
echo "  Cargo.toml            — Rust package"
echo "  src/main.rs           — WASM entry point (edit the TODO section)"
echo "  build-fixture.sh      — builds the WASM binary and prints the digest"
echo "  runtime-request.json  — test request for traverse-cli"
echo ""
echo "Next steps:"
echo "  1. Edit src/main.rs with your capability logic"
echo "  2. Edit contract.json: update description, input_schema, output_schema"
echo "  3. Build: bash ${OUTPUT_DIR}/build-fixture.sh"
echo "     (requires: rustup target add wasm32-wasip1)"
echo "  4. Inspect: cargo run -p traverse-cli-rs -- bundle inspect <path-to-bundle-manifest>"
echo ""
echo "For detailed guidance: docs/wasm-agent-authoring-guide.md"
