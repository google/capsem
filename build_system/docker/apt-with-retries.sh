#!/bin/bash
# Shared finite retry primitive for snapshot-backed image materializers.

capsem_apt_with_retry() {
    local operation_timeout="$1"
    shift
    local attempt=1
    local status=0
    while true; do
        if timeout --signal=TERM --kill-after=5s "${operation_timeout}s" "$@"; then
            return 0
        else
            status=$?
        fi
        if [ "$attempt" -ge "$APT_RETRIES" ]; then
            return "$status"
        fi
        echo "APT attempt $attempt/$APT_RETRIES failed; retrying snapshot request" >&2
        sleep $((attempt * 5))
        attempt=$((attempt + 1))
    done
}

capsem_apt_install_attempt_timeout() {
    local attempt_timeout=$((APT_INSTALL_TIMEOUT_SECONDS / APT_RETRIES))
    if [ "$attempt_timeout" -lt 1 ]; then
        echo "ERROR: apt install timeout must be at least the retry count" >&2
        return 1
    fi
    printf '%s\n' "$attempt_timeout"
}
