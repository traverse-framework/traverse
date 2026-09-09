# Web embedder npm publishing

The `traverse-embedder-web` package is released with npm Trusted Publishing.
GitHub Actions exchanges its OIDC identity for a short-lived npm credential;
no `NPM_TOKEN` or `NODE_AUTH_TOKEN` is stored in this repository.

## Release

1. Merge the package version and lockfile update to `main` after its tests pass.
2. From the merged commit, run `cd packages/web/TraverseEmbedder && npm version <version>`.
   The package `.npmrc` creates `web-v<version>` rather than npm's default `v` tag.
3. Push only that tag: `git push origin web-v<version>`.
4. The `Publish web embedder` workflow checks out the tagged commit, runs
   `npm ci`, verifies the tag matches `package.json`, builds, tests, and
   publishes with provenance.

Every release goes through this OIDC workflow. `0.9.0` is the first published
version (the earlier `0.8.0` bump was superseded on `main` before any npm
release — see `docs/decision-log.md` Decisions 76 and 77).

## Verification and recovery

After completion, run `npm view traverse-embedder-web version` and
`npm view traverse-embedder-web dist-tags --json`; confirm the expected version
is `latest`. Inspect its npm provenance in the package's npm registry page.

If a run is repeated after the same version has been published, it succeeds
without replacing the artifact. A tag/version mismatch fails before publishing.
If Trusted Publishing is not configured in npm for this repository and workflow,
the job fails safely; configure the npm trusted publisher as tracked by #1314,
then rerun the immutable tag workflow.
