# Model-signing key runbook (Decision 103, ADR-0080)

Traverse signs its published Spec 138 model packages with one dedicated
Ed25519 **model-signing key**. It is separate from the Registry
capability-artifact key (Spec 124).

- **Custody:** the private key exists only as the `MODEL_SIGNING_KEY_HEX`
  secret of the protected `model-signing` GitHub Environment, whose required
  reviewer is the maintainer.
- **Distribution:** public keys are committed under `keys/model-signing/`
  and mirrored into the opt-in `TRAVERSE_MODEL_SIGNING_KEYS` constant in
  both the Rust runtime and the web embedder.

## One-time setup (maintainer only)

1. **Generate** the key on a trusted machine, writing the secret
   **outside** the repository:
   ```bash
   node scripts/model-signing/generate-key.mjs --secret-out ~/model-signing.secret.hex
   ```
   It prints the `key_id` and writes the public key to
   `keys/model-signing/ed25519-<hex>.pub`.
2. **Create the environment.** In repo Settings → Environments, create
   `model-signing` with **Required reviewers: the maintainer**, restricted to
   the `main` branch.
3. **Upload the secret.** Add an environment secret named
   `MODEL_SIGNING_KEY_HEX` whose value is the contents of the secret file.
4. **Securely delete** the local secret file, for example with `rm -P`
   (macOS) or `shred -u` (Linux). If the secret is ever lost, that is a
   routine rotation.
5. **Commit the public key.** Commit the `.pub` file, add its 32 bytes to
   `TRAVERSE_MODEL_SIGNING_KEYS` in
   `crates/traverse-runtime/src/exact_model.rs` and its hex to
   `packages/web/TraverseEmbedder/src/exactModel.ts`, and open a PR. The
   mirror tests in both embedders fail until the constant and the directory
   agree.
6. **File the yearly rotation issue**, dated one year after the key's
   creation.

## Signing packages

1. Run **Actions → Sign model packages → Run workflow**. Set `packages` to
   the package directories under `fixtures/models/`, for example
   `digits-mlp-1.0.0`.
2. Approve the `model-signing` environment gate.
3. The job re-signs each package's exact manifest bytes, verifies it against
   the committed public keys, and pushes `model-signing/run-<id>`.
4. GitHub Actions can't open PRs in this repository, so **open the PR from
   that branch** yourself.
5. Update any conformance vector that pins the package (its `pin.key_id` and
   trusted key) in the same PR.

## Planned rotation (yearly, overlap window)

1. Generate the new key (setup steps 1, 4 and 5). **Add** it to
   `keys/model-signing/` and to both constants. The old key stays.
2. Replace the environment secret with the new key, then re-sign every
   published package with the workflow and merge.
3. Ship one minor release with both keys trusted. Hosts on that release
   accept old and new signatures.
4. In the next minor release, remove the old `.pub` file and constant
   entries.
5. File the next yearly rotation issue.

## Emergency revocation (compromise)

1. Generate a new key and replace the environment secret at once.
2. **Remove** the compromised `.pub` file and constant entries, add the new
   key, and re-sign every published package with the workflow.
3. Cut an **emergency patch release**. Upgraded hosts reject signatures from
   the old key with `key_untrusted` the next time they start.
4. Publish a **GitHub security advisory** naming the compromised `key_id`.

There is no denylist and no online revocation list (Decision 103); removing
the key from the shipped constant is the revocation.
