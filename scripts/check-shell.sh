#!/usr/bin/env bash
# ShellCheck over every tracked shell script.
#
# Python has Ruff and strict Ty, Rust has Clippy with warnings denied, and the
# web surfaces fail on warnings. Shell had nothing -- 6,821 lines across 46
# tracked scripts -- while four `# shellcheck disable=` directives sat in the
# tree, written for a linter no lane ran.
#
# `git ls-files` is the authority on what is first-party, the same rule the
# script size ratchet uses, so build output and vendored trees are excluded by
# construction rather than by pattern.
set -euo pipefail

SEVERITY="${1:?severity must be provided by the gate}"
IGNORE="${2:-}"

declare -a excluded=()
if [ -n "${IGNORE}" ]; then
    excluded=(--exclude "${IGNORE}")
fi

mapfile -t -d '' scripts < <(git ls-files -z -- '*.sh')
if [ "${#scripts[@]}" -eq 0 ]; then
    echo "no tracked shell scripts found; refusing to pass vacuously" >&2
    exit 1
fi

uv run shellcheck --severity="${SEVERITY}" "${excluded[@]}" -- "${scripts[@]}"
