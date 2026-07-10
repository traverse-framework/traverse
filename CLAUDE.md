# Traverse Development Guidelines

Auto-generated. Last updated: 2026-07-03

## Governance

This repo's constitution, NFRs, quality standards, antipatterns, compatibility policy, exception process, and CLA are **not** duplicated here — they live in [`traverse-framework/.github`](https://github.com/traverse-framework/.github), pinned at the version recorded in `.governance-version`. Read that repo's `constitution.md` before any implementation work.

Repo-specific product scope stays here: see `specs/001-foundation-v0-1/spec.md` for this repo's v0.1 scope, and `specs/051-registry-extraction/spec.md` for the in-progress registry extraction.

## Active Technologies

- Rust 1.94+
- Cargo workspace
- serde (JSON serialization)
- semver
- WASM (target)

## Project Structure

```text
crates/
  traverse-runtime/      # Core execution engine
  traverse-contracts/    # Contract definitions and validation
  traverse-registry/     # Capability and event registries (moving to traverse-framework/registry — see spec 051)
  traverse-cli/          # Command-line interface
  traverse-mcp/          # Model Context Protocol (stub)
specs/                   # Versioned, immutable governing specs
contracts/               # Capability and event contracts
docs/                    # Repo-specific docs (shared governance docs live in traverse-framework/.github)
.specify/                # Speckit: scripts, templates (constitution lives in traverse-framework/.github)
scripts/ci/              # Deterministic spec-alignment gate (vendored, pinned copy of traverse-framework/.github's version)
```

## Commands

```bash
cargo build              # Build all crates
cargo test               # Run tests (no panics, unwraps, or TODOs)
cargo run -p traverse-cli
bash scripts/ci/spec_alignment_check.sh   # Spec-alignment gate
```

## Code Style

- No `unsafe`, no `unwrap()`, no `panic!()`, no TODO in code
- 100% test coverage for core business and runtime logic
- Deterministic: same inputs must produce same outputs

## Development Workflow

1. Clarify capability boundary
2. Define or amend governing spec
3. Define contracts
4. Write tests
5. Implement smallest change satisfying spec + contract
6. Verify CI gate passes

All work must be tracked: GitHub issue + Project 1 item + PR.

## Approved Specs

Never list specs here — the registry is the source of truth:

```bash
jq -r '.specs[].id' specs/governance/approved-specs.json
```

<!-- MANUAL ADDITIONS START -->
<!-- MANUAL ADDITIONS END -->
