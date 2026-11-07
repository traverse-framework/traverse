#!/usr/bin/env bash

# Sourced. Points CARGO_TARGET_DIR at this checkout's own git-ignored target/
# when it points outside the checkout. Cargo reuses binaries across checkouts
# that share a target directory, and those binaries keep the paths of the
# checkout that built them, so checks could run another checkout's code (#1801).
# The build lives inside the checkout so it is removed with the checkout; a
# per-checkout folder under a shared directory outlived deleted worktrees.
# An unset value is left unset, and Cargo then uses target/ anyway.

checkout_target_dir_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
case "${CARGO_TARGET_DIR:-}" in
  "${checkout_target_dir_root}"/*) ;;
  /*) export CARGO_TARGET_DIR="${checkout_target_dir_root}/target" ;;
esac
unset checkout_target_dir_root
