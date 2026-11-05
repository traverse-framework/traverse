#!/usr/bin/env bash

# Sourced. Gives this checkout its own Cargo target directory when
# CARGO_TARGET_DIR points outside it. Cargo reuses binaries across checkouts
# that share a target directory, and those binaries keep the paths of the
# checkout that built them, so checks could run another checkout's code (#1801).
# The developer's chosen location is kept, and an unset value is left unset.

checkout_target_dir_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
checkout_target_dir_id="$(printf '%s' "${checkout_target_dir_root}" | git hash-object --stdin | cut -c1-16)"
case "${CARGO_TARGET_DIR:-}" in
  "${checkout_target_dir_root}"/* | */checkouts/"${checkout_target_dir_id}") ;;
  /*) export CARGO_TARGET_DIR="${CARGO_TARGET_DIR%/}/checkouts/${checkout_target_dir_id}" ;;
esac
unset checkout_target_dir_root checkout_target_dir_id
