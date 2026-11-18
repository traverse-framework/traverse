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

The workflow signs at a pinned commit (Decision 135, Spec 138 FR-076–FR-080):
either the head of an open PR to `main`, or the head of `main`.

### A new or changed package (normal case)

Any change to a production-signed manifest ships as a **new patch version**
in a new directory (for example `digits-mlp-1.0.1`); the old version stays
loadable for one minor release, then is removed.

1. Land any code changes first, in their own PR.
2. Open the **signing PR**. It may change only
   `fixtures/models/<package>/` and the package's entry in
   `scripts/fixtures/sign-model-fixtures.mjs`. Production rights text must
   not mention signing or keys; the generator refuses it. CI on this PR stays
   red until the signature is added, because the generator rejects an
   unsigned production package.
3. Run **Actions → Sign model packages → Run workflow** from `main`. Set
   `packages` to the package directories (for example `digits-mlp-1.0.1`) and
   `sha` to the signing PR's **full head commit SHA**.
4. Approve the `model-signing` environment gate.
5. Before the secret is used, the job refuses unless that SHA is the PR's
   current head and the PR's diff stays inside the listed packages and the
   generator. It then signs, verifies against the committed public keys, and
   pushes `model-signing/run-<id>` **on top of the PR's head**.
6. Fast-forward the PR branch to `model-signing/run-<id>` (or cherry-pick
   its one commit). The manifest and its signature merge together.
7. Move tests, conformance vectors and docs to the new version in a
   follow-up PR, and mark the old version superseded.

If the PR head moves after you dispatch, the run fails; dispatch again with
the new head.

### Re-signing what is already on `main` (key rotation)

Set `sha` to the current head of `main`. The job signs the packages as they
are on `main` and pushes `model-signing/run-<id>`. GitHub Actions can't open
PRs in this repository, so open the PR from that branch yourself, and update
any conformance vector that pins the package (its `pin.key_id` and trusted
key) in the same PR.

## Planned rotation (yearly, overlap window)

1. Generate the new key (setup steps 1, 4 and 5). **Add** it to
   `keys/model-signing/` and to both constants. The old key stays.
2. Replace the environment secret with the new key, then re-sign every
   published package that is not superseded, with `sha` set to the head of
   `main`, and merge.
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
