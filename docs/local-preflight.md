# Local CI Preflight

Run the required checks that are feasible on your machine before pushing a
pull request:

```bash
bash scripts/ci/local_preflight.sh --pr 809
```

Install the enforcement hook once per clone:

```bash
bash scripts/ci/install_pre_push_hook.sh
```

This fetches the PR body and base/head commits, verifies that the current
checkout is the PR head, and runs the same repository, Rust, WASM, coverage,
spec-alignment, and package-conformance scripts used by CI. It therefore
catches incorrect tests, fixture-build failures, digest drift, and missing
governing-spec declarations before another push.

For an unpublished change, save the intended PR body and provide its base
commit explicitly:

```bash
BASE_SHA=origin/main bash scripts/ci/local_preflight.sh --pr-body /path/to/pr-body.md
```

The command uses the CI Rust toolchain (currently 1.94.0) and requires its
`wasm32-unknown-unknown` target plus `cargo-llvm-cov`; it fails with the exact
install command when either is missing. On a Mac with Java and .NET installed
it also runs native artifact certification. It clearly reports the checks that
cannot be reproduced locally: CodeQL and the cross-platform stress matrix.

Because coverage and native certification can take several minutes, run this
as a deliberate pre-push gate instead of a fast pre-commit hook. After a PR
exists, `--pr <number>` is the safest form because it uses the exact body and
base commit that GitHub Actions will evaluate.
