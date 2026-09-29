#!/bin/bash
# This checkout-local path salts Cargo's workspace artifact hashes. Older
# snapshots cannot reuse another worktree's newer objects on mtime alone.
# Third-party crates retain their shared keys; compiler caching stays outside.
# Bash preserves Cargo's CARGO_BIN_EXE_<hyphenated-name> variables; dash drops them.
#
# The salt means every checkout keeps its own copy of each workspace unit, so
# each unit records which checkout compiled it: a symlink beside Cargo's
# fingerprint. Cache retention reclaims a unit whose checkout is gone and
# keeps the calling checkout's units last (capsem_builder.cache.cargounits).
checkout=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd -P)
out_dir=""
unit=""
previous=""
for argument in "$@"; do
  case "$previous" in
    --out-dir) out_dir=$argument ;;
    -C) case "$argument" in extra-filename=-*) unit=${argument#extra-filename=-} ;; esac ;;
  esac
  previous=$argument
done
case "$out_dir" in
  */deps) profile=${out_dir%/deps} ;;
  */build/*) profile=${out_dir%/build/*} ;;
  *) profile="" ;;
esac
if [ -n "$profile" ] && [ -n "$unit" ]; then
  for fingerprint in "$profile"/.fingerprint/*-"$unit"; do
    if [ -d "$fingerprint" ]; then
      ln -sfn "$checkout" "$fingerprint/capsem-owner" 2>/dev/null || true
    fi
  done
fi
exec "$@"
