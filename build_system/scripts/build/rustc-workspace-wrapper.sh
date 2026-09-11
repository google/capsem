#!/bin/bash
# This checkout-local path salts Cargo's workspace artifact hashes. Older
# snapshots cannot reuse another worktree's newer objects on mtime alone.
# Third-party crates retain their shared keys; compiler caching stays outside.
# Bash preserves Cargo's CARGO_BIN_EXE_<hyphenated-name> variables; dash drops them.
exec "$@"
