# Upgrading to Traverse v0.14.0

Governed by Spec `138-governed-exact-model-execution` (0.5.0), Decisions
101–102, and ADR-0077. See [docs/releases/v0.14.0.md](releases/v0.14.0.md)
for the full release narrative.

## Exact package versions

| Channel | Package | Version |
|---|---|---|
| crates.io | `traverse-runtime`, `traverse-embedder`, `traverse-contracts`, `traverse-mcp`, `traverse-cli-rs`, `traverse-expedition-wasm` | `0.14.0` |
| npm | `traverse-embedder-web` | `0.14.0` |
| Maven Central | `com.traverse-framework:traverse-embedder` | `0.14.0` |
| nuget.org | `TraverseEmbedder` | `0.14.0` |

These are lockstep per Decision 85. `traverse-registry` moves to `0.25.0`.

## Breaking: Spec 138 exact-ref model packages

Only apps that use `traverse.model-runtime` / `model.execute` exact-ref
models are affected.

1. **Manifest schema `2.0.0`.** `package_digest` is removed, and
   `license_id` / `attribution` / `redistribution` move into a required
   `rights` object that adds `commercial_use`
   (`allowed` | `restricted` | `prohibited`) and `source_url`. Unknown
   fields fail closed.
2. **Signature required.** Ship `model.sig.json`
   (`{ "alg": "ed25519", "key_id", "signature" }`): an Ed25519 signature
   over the **exact** manifest bytes, where
   `key_id = "ed25519:" + hex(sha256(public_key))`. Rust tooling can use
   `traverse_runtime::sign_model_manifest`.
3. **Pin digest.** An `exact_model_dependencies` pin's `digest` is now the
   SHA-256 of the exact manifest bytes (previously the WASM digest). Pins
   also require `target` (`wasm-cpu`) and `rights`
   (`{ license_id, commercial_use }`), and may add `key_id`.
4. **Host trust.** Configure trusted public keys. Apps can't add them.

   ```rust
   let mut keys = TrustedModelKeys::new();
   keys.trust(&public_key_bytes)?;
   let mut host = ExactModelHostConnector::new(pins, keys);
   host.register_package(&manifest_bytes, wasm, &signature_bytes)?;
   ```

   ```ts
   const host = new ExactModelBrowserHost(pins, { trustedPublicKeysHex: [publicKeyHex] });
   await host.registerPackage(manifestBytes, wasm, signatureBytes);
   ```

   `insertVerified` (web) and the public `ModelPackageStore::insert_verified`
   (Rust) are removed.
5. **Errors.** `HostConnectorError` / `ExactModelError` gain an optional,
   stable `reason`. `code` values are unchanged.
6. **Browser.** It requires WebCrypto Ed25519 (Chrome 137+, Firefox 129+,
   Safari 17+, Node 20+) and accepts only single exact-ref `wasm-cpu` pins.
   `execute` now returns `model_ref`, `target`, and `trace` alongside
   `output_ref` / `placement`.

## Platform support

Signed exact-ref model execution is implemented in the **Rust native
runtime** and the **web embedder**. The Swift, Kotlin, and .NET embedders
expose the `model.execute` command names but do not execute exact-ref
models yet.

## Trust keys

The packages in `fixtures/models/` are signed with a **test-only** key
(`fixtures/models/test-signing-key.json`). Never trust it in production.
Sign your own packages with your own key and configure that public key in
your host. The Traverse production model-signing key is #1567.
