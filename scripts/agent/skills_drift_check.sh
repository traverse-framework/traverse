#!/usr/bin/env bash
# Fail when the generated Claude/Cursor skill copies differ from .agents/skills
# (.github#58). Thin wrapper so the reusable governance workflow has one entry
# point; the logic lives in sync_skills.sh --check.
set -euo pipefail
exec bash "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/sync_skills.sh" --check
