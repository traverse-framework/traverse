---
name: traverse-app-builder
description: Builds apps with Traverse (traverse-framework/traverse) via Decision 80 plan-then-seal — clarify the goal, discover registry/MCP capabilities, run the declarative planner for candidates, author missing atomic capabilities when gaps remain, always pause for explicit seal confirmation, write sealed workflow.json + known_compositions, then CLI-validate (required). Use whenever the user wants to build a Traverse app/workflow/capability, compose capabilities, use traverse-cli, or run portable WASM business logic. Encodes the governed Host ABI (why ordinary wasm32-wasip1 builds fail) and the #![no_std] pattern that clears it.
---

# Building apps with Traverse (Decision 80)

Traverse's differentiator is **contracts**: every capability declares JSON-Schema
inputs/outputs, preconditions, postconditions, and permissions, and the runtime
enforces them. An app is usually reused registry capabilities + a few new atomic
ones + a **sealed** workflow that chains them.

**Product default (Decision 80):** plan at authoring time, then seal. What ships
and runs is a reviewed, pinned `workflow.json` referenced from the app/package
manifest (`known_compositions`). Runtime adaptive composition is **explicit
opt-in only** (see `#1345`) — never the default, and never silently persisted as
the app's sealed workflow.

Thin product pointer in-repo:
[`docs/workflow-authoring-guide.md`](../../../docs/workflow-authoring-guide.md).
This skill is the canonical deep ritual.

## Before you start

1. **Rust + `wasm32-unknown-unknown`** (`rustup target add wasm32-unknown-unknown`).
   Not `wasm32-wasip1` for guest capabilities — see the ABI section below.
2. **A clone of `traverse-framework/traverse`** for `traverse-cli` and examples.

**Homebrew/rustup PATH trap:** if both are installed, `which rustc` may point at
Homebrew and `wasm32-unknown-unknown` builds fail with missing `core`. Fix with
`PATH="$(dirname "$(rustup which cargo)"):$PATH"` — do not invent toolchain paths
from `uname -m` (`arm64` ≠ rustup's `aarch64-apple-darwin`).

## Decision 80 ritual (primary path)

Do this every authoring session unless the user already has a sealed graph and
only needs a validation/repair pass.

### 1. Clarify the goal

Capture a structured target: desired outcome facts / capability chain intent,
inputs available at start, and any hard constraints (targets, permissions).

### 2. Discover what already exists

```bash
traverse-cli registry sync --workspace <workspace-id> --json
# then read .traverse/workspaces/<workspace-id>/registry/public/index.json
```

The public index is a thin pointer (id, version, digests, URLs) — fetch
`contract_url` before deciding a candidate fits. For capabilities already in a
bundle: `traverse-cli capability discover <bundle-manifest> --json`.
Details: `references/registry.md`.

### 3. Plan candidates (planner is untrusted)

Use the declarative planner (spec `113`, ADR-0043 / ADR-0050). Prefer CLI
`workflow plan` / `workflow promote` when `#1346` has landed. Until then, call the
MCP library surface:

- `traverse_mcp::tools::workflow_plan::plan_workflow` (candidate graphs)
- `traverse_mcp::tools::workflow_promotion::{export_workflow_candidate, ...}`
  after a reviewed proposal (spec `112`)

Show **all** returned candidates (or the truncation notice). Do not pick one
silently. Do not treat a plan as a runnable app workflow.

### 4. Close gaps, then re-plan

If published capabilities cannot complete the goal:

1. Author each missing **atomic** capability (contract → ABI-clean WASM →
   manifest) using the sections below.
2. Re-run planning against the updated set.
3. Refuse to seal placeholder or partial workflows.

### 5. Human seal gate (mandatory)

**Always pause** and ask for an explicit “seal this proposal” (or equivalent)
before writing `workflow.json` / manifest refs. Show the chosen graph:
capability ids + versions, edges, and any newly authored caps.

- Refuse auto-seal when exactly one candidate exists.
- Refuse seal without confirmation.
- Multi-candidate sessions must present the choice, then wait.

### 6. Write sealed artifacts

After confirmation:

- Write `workflow.json` (`kind: "workflow_definition"`) with pinned nodes/edges.
- Reference it from the package/app via `known_compositions` (prefer over legacy
  `workflow_refs`; if both are present they must match).
- Fix digests when the CLI reports mismatches (`fnv1a64:...`).

### 7. Session DoD (required vs optional)

**Required**

```bash
cargo run -p traverse-cli-rs -- app validate --manifest <app-or-package-manifest> --json
# and/or agent inspect / workflow register+inspect as appropriate to the artifact
```

Success = artifacts on disk after confirm **and** validation green.

**Optional follow-on** (offer when the workspace/runtime is ready): register + one
smoke execute. Honest execute surfaces today:

- Single capability: `traverse-cli agent execute <manifest> <request>`
- Workflow: no bare `workflow execute`; use `traverse-cli serve` with a
  composite/workflow capability, or domain helpers like `expedition execute`
- There is still no generic multi-target parity command

## Authoring one new atomic capability

Keep this path for gaps and single-capability work. Prefer atomic verbs — if the
user names two verbs, that is usually two capabilities + a workflow.

### 1. Name and shape

Capability id: `namespace.name`. Decide JSON Schema inputs/outputs first.

### 2. Contract + single-node workflow

Copy shapes from `references/contract-schema.md` (derived from
`CapabilityContract` in `crates/traverse-contracts`). **Do not** use
`scripts/scaffold/new-capability.sh` templates — they were verified stale
(`input_schema`/`output_schema` flat keys, missing required fields).

Set `lifecycle` to `"active"` when you intend to run it (`"draft"` is rejected as
not runnable). Packages need at least one composition reference
(`known_compositions`).

### 3. Implement with the Host ABI whitelist

`WasmExecutor` allows only `wasi_snapshot_preview1::{fd_read, fd_write, proc_exit}`
plus a small `traverse_host` set (`crates/traverse-runtime/src/executor/host_abi_v1.json`).

Ordinary `cargo build --target wasm32-wasip1` fails ABI verify because Rust's
WASI startup imports `environ_get`. The verified pattern is `#![no_std]` +
hand-declared WASI imports, built with `rustc --target wasm32-unknown-unknown
--crate-type cdylib`.

```rust
#![no_std]
#![no_main]

#[repr(C)]
struct IoVec { buffer: *const u8, length: usize }
#[repr(C)]
struct IoVecMut { buffer: *mut u8, length: usize }

#[link(wasm_import_module = "wasi_snapshot_preview1")]
unsafe extern "C" {
    fn fd_read(fd: u32, vectors: *const IoVecMut, count: usize, read: *mut usize) -> u32;
    fn fd_write(fd: u32, vectors: *const IoVec, count: usize, written: *mut usize) -> u32;
}

static mut INPUT_BUF: [u8; 4096] = [0; 4096];
static mut OUTPUT_BUF: [u8; 4096] = [0; 4096];

#[unsafe(no_mangle)]
pub extern "C" fn _start() {
    unsafe {
        let mut read = 0usize;
        let mut ivec = IoVecMut { buffer: INPUT_BUF.as_mut_ptr(), length: INPUT_BUF.len() };
        let _ = fd_read(0, &ivec, 1, &mut read);
        // parse/transform INPUT_BUF by hand into OUTPUT_BUF, then:
        let mut written = 0usize;
        let out = IoVec { buffer: OUTPUT_BUF.as_ptr(), length: /* n */ 0 };
        let _ = fd_write(1, &out, 1, &mut written);
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! { loop {} }
```

Costs: no `serde_json`/heap unless you bring an allocator; `static mut` needs care;
panics loop until fuel kills the instance. Prefer defensive validation.

### 4. Compile and ABI-verify

```bash
rustc src/agent.rs --target wasm32-unknown-unknown --crate-type cdylib -O -o artifacts/your-agent.wasm
traverse-cli wasm abi verify artifacts/your-agent.wasm
```

### 5. Inspect / digest-fix

```bash
cargo run -p traverse-cli-rs -- agent inspect path/to/manifest.json
```

Paste the reported `fnv1a64:` digest into `binary.expected_digest`, re-inspect.
`agent inspect` does **not** check host imports — ABI verify still matters.

## Manual composition (when the graph is already known)

Hand-authored multi-node workflows remain valid. Prefer the Decision 80 planner
path when discovery or gap-authoring is part of the session. Annotated shapes and
the `from_workflow_input` state-accumulation gotcha live in
`references/contract-schema.md`.

```bash
traverse-cli workflow register path/to/workflow.json --workspace <workspace-id>
traverse-cli workflow list --workspace <workspace-id>
traverse-cli workflow inspect <workflow-id> --workspace <workspace-id>
```

## Multi-target honesty

Same WASM + `Runtime<E: LocalExecutor>` is the portability mechanism. Shipped
today: native (Rust embedder) and browser (Web embedder). Swift/iOS is blocked on
upstream WasmKit; Kotlin/.NET exist but are not the default release path; edge is
unstarted; cloud placement is a non-goal for v0.1. There is no single parity
command — prove two embedders yourself. See `references/platforms.md`.

## Where to go deeper

- `references/registry.md` — discovery and publish
- `references/contract-schema.md` — contract/workflow/manifest/request shapes
- `references/platforms.md` — per-target status
- `references/troubleshooting.md` — verbatim errors and fixes
- `docs/workflow-authoring-guide.md` — sealed-default vs adaptive opt-in
- `docs/capability-contract-authoring-guide.md`, `docs/wasm-agent-authoring-guide.md`

## Personal copy note

If a personal `~/.claude/skills/traverse-app-builder` exists, prefer this in-repo
copy (`.agents/skills/traverse-app-builder/`) or keep the personal copy in sync
with it after refreshes.
