#!/usr/bin/env bash
set -euo pipefail

: "${WORKSPACE_APT_PACKAGES:?native apt packages are required}"
: "${WORKSPACE_CROSS_APT_PACKAGES:?cross apt packages are required}"
: "${APT_SNAPSHOT_ID:?apt snapshot ID is required}"
: "${APT_UPDATE_TIMEOUT_SECONDS:?apt update timeout is required}"
: "${APT_INSTALL_TIMEOUT_SECONDS:?apt install timeout is required}"
: "${APT_REQUEST_TIMEOUT_SECONDS:?apt request timeout is required}"
: "${APT_RETRIES:?apt retry count is required}"
read -r -a native_packages <<<"$WORKSPACE_APT_PACKAGES"
read -r -a cross_packages <<<"$WORKSPACE_CROSS_APT_PACKAGES"

rm -f /etc/apt/apt.conf.d/docker-clean
rm -f /var/lib/apt/lists/lock /var/cache/apt/archives/lock

marker="/var/lib/apt/lists/.capsem-${APT_SNAPSHOT_ID}-complete"
if [[ ! -f "$marker" ]]; then
    attempt=0
    while ! timeout --signal=TERM --kill-after=5s "${APT_UPDATE_TIMEOUT_SECONDS}s" apt-get \
        -o Acquire::https::Timeout="${APT_REQUEST_TIMEOUT_SECONDS}" \
        -o Acquire::Retries="${APT_RETRIES}" update; do
        attempt=$((attempt + 1))
        [[ "$attempt" -lt "$APT_RETRIES" ]] || exit 1
    done
    touch "$marker"
fi

attempt=0
while ! timeout --signal=TERM --kill-after=5s \
    "$((APT_INSTALL_TIMEOUT_SECONDS / APT_RETRIES))s" apt-get \
    -o Acquire::https::Timeout="${APT_REQUEST_TIMEOUT_SECONDS}" \
    -o Acquire::Retries="${APT_RETRIES}" install -y --no-install-recommends \
    "${native_packages[@]}" \
    "${cross_packages[@]}" \
    curl wget ca-certificates git sqlite3 python3-venv; do
    attempt=$((attempt + 1))
    [[ "$attempt" -lt "$APT_RETRIES" ]] || exit 1
done
