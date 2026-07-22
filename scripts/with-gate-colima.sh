#!/bin/bash
# Restore Colima to the state it had before an expensive local release gate.
# If the developer already had Colima running, the gate leaves it alone. If
# bootstrap starts it for the gate, this wrapper stops it on success or error.

set -euo pipefail

colima_was_running=0
if command -v colima >/dev/null 2>&1 && colima status >/dev/null 2>&1; then
    colima_was_running=1
fi

cleanup_gate_colima() {
    status=$?
    trap - EXIT
    if [ "$colima_was_running" -eq 0 ] \
        && command -v colima >/dev/null 2>&1 \
        && colima status >/dev/null 2>&1; then
        echo "=== Stopping gate-owned Colima VM ==="
        if ! colima stop; then
            echo "WARNING: failed to stop Colima started by this gate" >&2
        fi
    fi
    exit "$status"
}

trap cleanup_gate_colima EXIT
"$@"
