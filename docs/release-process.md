# Traverse Release Process

Governed by spec `048-semver-publishing-pipeline`.

Use the release helper from a clean `main` checkout after CI is green:

1. Run `bash scripts/ci/bump_version.sh <version>`.
2. Run `git push origin main`.
3. Run `git push origin v<version>`.
4. Run `git push origin web-v<version>`.
5. Confirm the CI publish job starts automatically from the `v<version>` tag
   push, and the web embedder Trusted Publishing workflow starts from the
   `web-v<version>` tag push.
6. Verify crates.io lists the Traverse-published crates at the new version:
   `traverse-contracts`, `traverse-runtime`, `traverse-mcp`, `traverse-cli-rs`,
   and `traverse-expedition-wasm`. `traverse-registry` is released separately
   from `traverse-framework/registry`.
7. Verify npm lists `traverse-embedder-web@<version>`
   (`npm view traverse-embedder-web version`).

The version argument is `MAJOR.MINOR.PATCH` without a leading `v`. The helper
refuses invalid versions, dirty working trees, and pre-existing local release
tags (`v*` or `web-v*`). It rewrites versions in four files and no others:
`[workspace.package] version` (plus the matching `[workspace.dependencies]`
path-crate `version =` pins) in `Cargo.toml`; the `version = "…"` line of
every workspace path crate in `Cargo.lock` (`[[package]]` entries named
`traverse-*` with no `source` line; registry dependencies such as
`traverse-registry` are left untouched); and both
`packages/web/TraverseEmbedder/package.json` and `package-lock.json` (root and
`packages[""]`). All four files land in one `chore: bump version to v<version>`
commit, and local tags `v<version>` and `web-v<version>` are created. It does
not push commits or tags.

Keeping `Cargo.lock` in step matters: a lockfile left at the old version breaks
every `--locked` build and blocks `cargo publish`. After running the helper,
`cargo metadata --locked` must succeed with no lockfile change.

Crate and npm versions are lockstep: every Traverse release `X.Y.Z` publishes
crates `X.Y.Z` and `traverse-embedder-web@X.Y.Z`. Web is not an independent
version line (Decision 85).

On a `v*` tag push, the `version-guard` CI job compares the tag without the
leading `v` to the workspace version in `Cargo.toml` and to web
`package.json`. Branch and pull-request runs pass without a release tag, while
mismatched release tags fail before publishing. The same job also runs
`cargo metadata --locked` on every push, pull request, and tag, so a
`Cargo.toml`/`Cargo.lock` version drift fails CI before it can reach the
tag-only `publish` job.

The tag-only `publish` CI job runs after `version-guard`, repository checks, and
coverage pass. It runs `bash scripts/ci/publish_crates.sh` with
`CARGO_REGISTRY_TOKEN` from GitHub Actions secrets. The script dry-runs each
crate immediately before publishing it, publishes crates in dependency order,
and treats an already-uploaded crate version as success so reruns are
idempotent.

## Web embedder (npm)

The web SDK releases with the same `X.Y.Z` as the crate cut. The bump helper
already updates the web package/lockfile and creates `web-v<version>`. Push
that tag after `main` to trigger the credential-free npm Trusted Publishing
workflow. See [the web embedder npm publish runbook](web-embedder-npm-publish-runbook.md)
for setup assumptions and verification.

## Post-cut documentation, READMEs, and website

`bump_version.sh` only rewrites Cargo / web package version fields. After
crates.io and npm list the new `X.Y.Z`, finish the human-facing surfaces:

### 1. This repo (before or with the release-notes PR; finish after publish)

- [ ] `docs/releases/vX.Y.Z.md` — full release notes (move content out of
      `docs/releases/next.md`).
- [ ] `CHANGELOG.md` — matching `## vX.Y.Z` section; clear Unreleased.
- [ ] Root `README.md` — version badge, Install pins (`cargo add` /
      `npm install`), Project state table, “current release notes” links.
- [ ] Package READMEs that pin a published version (at least
      `packages/web/TraverseEmbedder/README.md`).
- [ ] Example / consume READMEs that hard-code a version, if any.
- [ ] Approved-spec count in README if `specs/governance/approved-specs.json`
      changed since the last cut
      (`jq -r '.specs | length' specs/governance/approved-specs.json`).
- [ ] Native runtime artifact registry: if this cut ships a new certified
      `runtime.wasm`, publish a new `runtime_version` row in
      `runtime/native-runtime-registry.json` (spec `075` FR-007 forbids
      overwriting). See [native-runtime-artifact-registry.md](native-runtime-artifact-registry.md).

### 2. Website (`traverse-framework/website`)

File a website issue before the cut (pattern:
[website#91](https://github.com/traverse-framework/website/pull/91) /
[website#97](https://github.com/traverse-framework/website/issues/97) for
v0.11.0). After publish is live:

- [ ] Truth-pass every current-version surface (hero, footer, system, about,
      roadmap, faq, quickstart, concepts, production-ready Q, platforms as
      needed) to `vX.Y.Z`.
- [ ] Changelog page: new latest entry distilled from the GitHub Release —
      no invented features.
- [ ] Short release blog when the ship story is user-visible.
- [ ] Re-verify live registry counts / approved-spec count before claiming
      them.
- [ ] `npm run build` and `npm test` green in the website repo.

### 3. Announcement

- [ ] GitHub Release notes from `docs/releases/vX.Y.Z.md`.
- [ ] Org Discussion (or linked announcement) when the cut is public.
