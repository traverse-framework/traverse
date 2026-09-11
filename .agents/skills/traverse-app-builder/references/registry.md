# Checking the registry before you build anything new

Traverse's registry is the mechanism for reuse: a capability someone else already wrote, contracted, and published should be something you can find and call, not something you re-implement. Before scaffolding a new capability, check whether one already covers what you need — the commands below are real and verified against `crates/traverse-cli/src/main.rs` and `crates/traverse-registry/src/public_registry_state.rs`, not inferred from docs copy.

## Two different registries, two different commands

Don't conflate these — they answer different questions:

1. **"What's published publicly, across the ecosystem?"** → `registry sync` + reading the synced index.
2. **"What capabilities did I already declare inside this specific app bundle I'm assembling?"** → `capability discover`.

Neither is a live network search. Both work against local state — the runtime and CLI never live-fetch during normal use, which is a deliberate determinism choice, not a limitation to work around.

## 1. Sync and read the public registry index

```bash
traverse-cli registry sync --workspace <workspace-id> --json
```

This fetches the latest public registry index from `traverse-framework/registry` and writes it to `.traverse/workspaces/<workspace-id>/registry/public/index.json` under your current directory. The command's own JSON output is intentionally thin — `status`, `source`, `release_tag`, `record_count`, `synced_at`, `state_path` — it tells you *that* the sync worked and *how many* records came down, not what they are.

**To actually see what's available, read the state file it just wrote:**

```bash
cat .traverse/workspaces/<workspace-id>/registry/public/index.json
```

Each record has this shape (verified against `PublicRegistryCapabilityRecord` in `crates/traverse-registry/src/public_registry_state.rs`):

```json
{
  "namespace": "some-namespace",
  "id": "some-namespace.some-capability",
  "version": "1.0.0",
  "digest": "fnv1a64:...",
  "artifact_url": "https://.../artifact.wasm",
  "contract_digest": "sha256:...",
  "contract_url": "https://raw.githubusercontent.com/traverse-framework/registry/.../contract.json",
  "deprecated": false
}
```

**This is a thin pointer, not a catalog entry** — notice there's no `summary`, `description`, `inputs`, or `outputs` field. The public index only carries enough to identify and verify a capability (id, version, digest, URLs), not enough to know what it actually does. To evaluate whether an existing capability fits your need, fetch `contract_url` and read the real contract — the same shape documented in `references/contract-schema.md`.

## 2. Check what's already declared in an app bundle you're assembling

If you're working inside an app's own registry bundle (a local `manifest.json` that lists the capabilities/events/workflows an app assembles, distinct from the thin public index above):

```bash
traverse-cli capability discover path/to/registry-bundle/manifest.json --json
```

This loads that bundle into an in-memory registry and lists everything actually registered in it — id, version, scope, lifecycle, `implementation_kind` (`executable` vs `workflow`), summary, and tags. This is richer than the public index because it's built from full contracts already resolved into memory, but it only reflects what that one bundle declares — not the wider public registry, and not anything from a `registry sync` unless the bundle was registered with `bundle register` first (which merges in synced public records; `capability discover` alone does not).

## 3. Publishing an atomic capability you built

Once your capability validates cleanly (`agent inspect` passes — see SKILL.md steps 2-5), making it available to others is a real, if deliberately unautomated, flow:

```bash
traverse-cli capability publish \
  --contract path/to/contract.json \
  --artifact path/to/artifact.wasm \
  --registry-repo path/to/local/traverse-framework/registry-checkout \
  --json \
  [--dry-run]
```

This validates the contract and artifact digest, then prepares a publication candidate under a local checkout of `traverse-framework/registry` — **it never pushes or opens the PR for you automatically without a human in the loop**; the command's own help text is explicit that it "opens a human-reviewed registry PR," not that it merges anything. Always run with `--dry-run` first to see the planned branch and paths before doing it for real.

## Workflow-side registry commands (real, but registry-only — not execution)

`workflow register` / `workflow list` / `workflow inspect` mirror the capability commands for workflow definitions — they're genuinely generic and work for any workflow you throw at them, not demo-scoped. But like the capability commands above, they only cover registration and inspection. There is no generic `workflow execute` in the CLI — see SKILL.md's "Actually running it" section for why, and what to do about it if you need a workflow to actually run end-to-end.
