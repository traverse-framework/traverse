# Local CI Preflight

Run the locally feasible CI checks before pushing a PR:

```bash
BASE_SHA=origin/main bash scripts/ci/local_preflight.sh --pr-body /path/to/pr-body.md
```

The command uses the same repository, Rust, WASM smoke, and spec-alignment
scripts as CI. It reports hosted-only jobs explicitly instead of claiming they
ran locally.

To make the Rust subset mandatory for every local commit, enable the tracked
hook once per clone:

```bash
git config core.hooksPath .githooks
```

The hook runs `scripts/ci/rust_checks.sh`, the same formatter, strict Clippy,
workspace-test, and contractual-enforcement command used by CI.
