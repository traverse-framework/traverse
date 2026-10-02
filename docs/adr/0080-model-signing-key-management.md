# ADR-0080: Production Model-Signing Key Management

- Status: Accepted
- Date: 2026-10-01
- Governing specs: `138-governed-exact-model-execution` (0.11.0)
- Related issues: `#1567`
- Related: `docs/decision-log.md` Decision 103 (custody, distribution,
  rotation, revocation); Decision 101 (format, host-owned trust)
- Owner: Traverse maintainers

## Context

Decision 101 fixed the package signature format, a detached Ed25519
signature over the exact manifest bytes, and made trust host-owned. Every
Traverse package was still signed with a committed test-only key, so none
was publishable. Decision 103 chose how the production key is held,
distributed, used, rotated, and revoked.

## Decision

1. **Custody.** One dedicated Ed25519 model-signing key, separate from the
   Spec 124 Registry key. Its secret lives only in the protected
   `model-signing` GitHub Environment as `MODEL_SIGNING_KEY_HEX`.
2. **Use.** Only `.github/workflows/sign-model-packages.yml` uses the secret.
   That workflow runs on `workflow_dispatch` only, in that environment. It
   calls `scripts/model-signing/sign-packages.sh`, which writes the secret to
   a 0600 temp file that is removed on exit, signs with
   `traverse-cli model sign`, and verifies with `traverse-cli model verify`
   against the committed public keys. The secret is never available to
   `pull_request` workflows.
3. **Generation.** `scripts/model-signing/generate-key.mjs` refuses to write
   the secret inside the repository, writes it with mode 0600, and writes
   the public key to `keys/model-signing/ed25519-<hex>.pub`, named after its
   `key_id`.
4. **Distribution.** `TRAVERSE_MODEL_SIGNING_KEYS` (Rust `&[[u8; 32]]`,
   web `string[]`) mirrors `keys/model-signing/` exactly, and a test in each
   embedder enforces it. Trust is opt-in: there is no default trust and no
   network discovery.
5. **Signing PRs.** GitHub Actions may not create pull requests here, so the
   workflow pushes `model-signing/run-<id>` and the maintainer opens the PR.
6. **Rotation and revocation** follow Decision 103: an overlap window of one
   minor release, yearly or on compromise, and revocation by an emergency
   patch release plus a security advisory. The procedure is in
   `docs/model-signing-key-runbook.md`.

## Consequences

- No package is production-signed until the maintainer generates the key,
  creates the environment, and commits the public key. Re-signing
  `digits-mlp-1.0.0` is the follow-up after that.
- A workflow run with a secret whose public key is not committed fails
  verification (`key_untrusted`) instead of producing unpublished
  signatures.

## Alternatives Considered

See Decision 103: a maintainer-held offline key, Sigstore keyless signing,
reusing the Registry key, built-in default trust, multi-signature packages,
a denylist, and an online revocation list. Opening the signing PR from CI
was not possible because of a repository setting.

## Approval

Approved by Enrico in the Decision 103 `/brainstorm` (2026-09-29).
