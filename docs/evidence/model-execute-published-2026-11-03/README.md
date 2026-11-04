# Catalog `model.execute` on published v0.16.1 packages (#1776, #1719)

- **Date:** 2026-11-03
- **Issues:** `#1776`, `#1719`, leftover demo requirement of `#1655`
- **Governing specs:** `1719-suspended-connector-invocation`, `138-governed-exact-model-execution`, `104-mediated-connector-invocation`
- **Result:** [`proof/comparison.json`](proof/comparison.json) reports `all_identical: true`

## Result

The published `model/model.execute@1.0.0` guest, unchanged, ran over the signed
`traverse.digits-onnx@1.0.0` package on three hosts:

| Host | Installed from | Engine |
|---|---|---|
| Native | crates.io `traverse-runtime` 0.16.1 | wasmtime |
| Node 24.16.0 | npm `traverse-embedder-web@0.16.1` | nested wasmi in `runtime.wasm` |
| Chromium 152 | the same npm package, served from `127.0.0.1` | nested wasmi in `runtime.wasm` |

The success output, the 56-byte output frame and every failure below are
identical on all three. Native also adds `placement_evidence`, which Spec 1719
FR-014 allows. The frame also equals the signed vector's expected output. On
native, the one-call `activate_exact_model_runtime` binding gives the same output.

| Case | Setup | Registration reason | Capability output |
|---|---|---|---|
| `success` | none | none | `status: ok` with `model_evidence` |
| `connector_unavailable` | connector not activated | none | `reason: connector_unavailable` |
| `usage_undeclared` | host declares no model usage | `usage_undeclared` | `model_unavailable`, package not in the verified cache |
| `key_untrusted` | no trusted keys | `key_untrusted` | `model_unavailable`, package not in the verified cache |
| `pin_mismatch` | pin names another digest | `pin_mismatch` | `model_unavailable` / `pin_mismatch` |
| `digest_mismatch` | one byte of `model.wasm` flipped | `digest_mismatch` | `model_unavailable`, package not in the verified cache |
| `package_revoked` | status map revokes after registration | none | `model_unavailable` / `package_revoked` with denial detail |

## Digests

| Artifact | Value |
|---|---|
| npm `traverse-embedder-web@0.16.1` integrity | `sha512-tZnGmE1bqiVHd2bbSBQjSZYXhFTbBjc84akvNZXLMeDXtau569vc04U5Pc3PqXsicV1++5wGO1cYbimL4WYngQ==` |
| crate `traverse-runtime` 0.16.1 | `3f0c8e5797f95775230c4d5ab42ce42d34ecc1b554be220614695cb3ae1db20c` |
| crate `traverse-contracts` 0.16.1 | `7cc964f74c5aae0dbf24b25b149976cccaace0e3c104543dd288d77e5b67d4cb` |
| crate `traverse-registry` 0.29.0 | `f6dcd99577f7e7d738654272f4879036db3a335faa73bf276ab7d38243dffea4` |
| `runtime.wasm` | `7a322a17e3fd58f8bd26e915dd81bad68732500dc981499bd280a6d2e6507d52` |
| `model-execute-agent.wasm` | `5ccfa3721db61b42a598d703c2a110c57beaf57cde26e1c5cb7f9c5fbc8b4275` |
| Package digest (`model.manifest.json`) | `a7a8aa35c79ed38e7e2646d1b4f531e5d3167949ac8bd3e4efc8bfd678b901d2` |
| `model.wasm` | `954ca59b005699efcd126cde69446c7a935bfccd9561dd248ddd844246ea10a9` |

`traverse-mcp`, Maven, NuGet and Swift were not used.

## Reproduce

Copy this folder anywhere outside a Traverse checkout. You need Node and Rust
1.95 or later.

```bash
bash run.sh
```

```bash
node src/server.mjs
```

Open `http://127.0.0.1:8791/` and wait for the page to say `DONE`. Then:

```bash
node src/compare.mjs
```

The committed proof comes from a second run of these steps, in a new empty
folder outside any git repository, with a blank `HOME` and fresh npm and Cargo
caches. A first run in another empty folder gave the same pass.

## Limits

- **Not a separate clean machine.** Both runs were on one Mac that has Traverse
  checkouts elsewhere. Neither run read from them, but nothing enforces that.
- **The real browser is Chromium inside the Claude desktop app**, not a separate
  Chrome install. It ran once per folder, opened by hand.
- **Two inputs come from the tag, not a package or release:**
  - `runtime.wasm`, fetched by URL from the `v0.16.1` tag. Its digest matches
    `runtime/runtime-release.json`. npm doesn't ship it, and the last
    `runtime-wasm-v*` release is v0.11.0, although `docs/release-process.md` lists
    that release type.
  - The `digits-onnx` package, also from the tag. It is signed with the
    **test-only** key.
- **Output comparison:** the hosts expose parsed output, so the comparison uses
  sorted-key JSON, not raw stdout bytes. Output frames are compared byte for byte.
- **Frame access on native:** `activate_exact_model_runtime` keeps the host, so a
  caller can't read the output frame back through it. The frame check uses
  `ExactModelMediatedConnector` directly.
- Swift, Android and .NET aren't covered. Their READMEs say the binding isn't
  shipped yet.
