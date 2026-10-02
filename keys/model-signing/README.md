# Production model-signing public keys (Decision 103)

Each `ed25519-<sha256 of the raw public key>.pub` file holds one production
Spec 138 model-signing **public** key as 64 lowercase hex characters.

- The file name is the key's `key_id` (`ed25519:<hex>`) with `:` replaced by
  `-`, because `:` is not portable in file names.
- `TRAVERSE_MODEL_SIGNING_KEYS`, in both the Rust runtime and the web
  embedder, mirrors this directory exactly; a test in each enforces that.
- Hosts opt in by passing those keys as trusted keys. There is no default
  trust and no network discovery.
- Private keys never live here. The production secret exists only as the
  `MODEL_SIGNING_KEY_HEX` secret of the protected `model-signing` GitHub
  Environment.

See [`docs/model-signing-key-runbook.md`](../../docs/model-signing-key-runbook.md)
for generation, rotation, and revocation.
