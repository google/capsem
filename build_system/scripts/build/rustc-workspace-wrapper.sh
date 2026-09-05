#!/bin/sh
# This checkout-local path salts Cargo's workspace artifact hashes. Older
# snapshots cannot reuse another worktree's newer objects on mtime alone.
# Third-party crates retain their shared keys; compiler caching stays outside.
exec "$@"
