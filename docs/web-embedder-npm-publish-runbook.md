# Web embedder npm publishing

The `traverse-embedder-web` package is released with npm Trusted Publishing.
GitHub Actions exchanges its OIDC identity for a short-lived npm credential;
no `NPM_TOKEN` or `NODE_AUTH_TOKEN` is stored in this repository.

Crate and npm versions are lockstep (Decision 85 / spec 048): every Traverse
release `X.Y.Z` publishes crates `X.Y.Z` and `traverse-embedder-web@X.Y.Z`.

## Release

1. From a clean, green `main` checkout, run
   `bash scripts/ci/bump_version.sh <version>`.
   That updates Cargo and the web package/lockfile in one commit and creates
   local tags `v<version>` and `web-v<version>`.
2. Push the bump commit: `git push origin main`.
3. Push both tags:
   `git push origin v<version>` and `git push origin web-v<version>`.
4. The `Publish web embedder` workflow checks out the tagged commit, runs
   `npm ci`, verifies the tag matches `package.json`, verifies the Cargo
   workspace version matches the same version, builds, tests, and publishes
   with provenance.

Do not cut an independent web-only version with `npm version`; the helper is
the dual-tag path.

Every release goes through this OIDC workflow. `0.9.0` was the first published
version (the earlier `0.8.0` bump was superseded on `main` before any npm
release — see `docs/decision-log.md` Decisions 76 and 77). From `0.10.2`
onward, npm tracks the crate line.

## Verification and recovery

After completion, run `npm view traverse-embedder-web version` and
`npm view traverse-embedder-web dist-tags --json`; confirm the expected version
is `latest`. Inspect its npm provenance in the package's npm registry page.

If a run is repeated after the same version has been published, it succeeds
without replacing the artifact. A tag/version mismatch, or a Cargo/web version
mismatch, fails before publishing. If Trusted Publishing is not configured in
npm for this repository and workflow, the job fails safely; configure the npm
trusted publisher as tracked by #1314, then rerun the immutable tag workflow.
