#!/usr/bin/env bash
set -euo pipefail

: "${WORKSPACE_APT_PACKAGES:?native apt packages are required}"
: "${WORKSPACE_CROSS_APT_PACKAGES:?cross apt packages are required}"
: "${APT_SNAPSHOT_ID:?apt snapshot ID is required}"
: "${APT_UPDATE_TIMEOUT_SECONDS:?apt update timeout is required}"
: "${APT_INSTALL_TIMEOUT_SECONDS:?apt install timeout is required}"
: "${APT_REQUEST_TIMEOUT_SECONDS:?apt request timeout is required}"
: "${APT_RETRIES:?apt retry count is required}"
: "${APT_DOWNLOAD_CONCURRENCY:?apt download concurrency is required}"
: "${APT_SNAPSHOT_BASE:?apt snapshot base is required}"
[[ "$APT_DOWNLOAD_CONCURRENCY" =~ ^[1-9][0-9]*$ ]] && \
    [[ "$APT_DOWNLOAD_CONCURRENCY" -le 16 ]] || exit 1
read -r -a native_packages <<<"$WORKSPACE_APT_PACKAGES"
read -r -a cross_packages <<<"$WORKSPACE_CROSS_APT_PACKAGES"

rm -f /etc/apt/apt.conf.d/docker-clean
rm -f /var/lib/apt/lists/lock /var/cache/apt/archives/lock
# A fresh BuildKit parent cache hides the image's archive directories.
mkdir -p /var/cache/apt/archives/partial

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

# APT serializes requests to one host. Hundreds of snapshot requests exhausted
# all three retry windows despite retaining most archives. Resolve once from
# authenticated indexes, then let independent APT workers fetch bounded batches.
# No shell evaluation of the printed manifest, alternate archive or weak hash.
materialize_archives() {
    set -euo pipefail
    manifest=$(mktemp)
    trap 'rm -f "$manifest"' EXIT
    apt-get -o Acquire::ForceHash=SHA256 --print-uris --yes \
        --no-install-recommends install "$@" > "$manifest"
    pids=()
    failed=0
    while read -r uri filename size digest extra; do
        [[ "$uri" == "'"* ]] || continue
        uri=${uri#\'}
        uri=${uri%\'}
        if [[ "$uri" != "${APT_SNAPSHOT_BASE%/}/${APT_SNAPSHOT_ID}/"* ||
              ! "$filename" =~ ^[[:alnum:]_.+%:-]+\.deb$ ||
              ! "$size" =~ ^[0-9]+$ ||
              ! "$digest" =~ ^SHA256:[a-fA-F0-9]{64}$ || -n "$extra" ]]; then
            echo "invalid authenticated APT archive manifest" >&2
            failed=1
            break
        fi
        /usr/lib/apt/apt-helper \
            -o Acquire::https::Timeout="${APT_REQUEST_TIMEOUT_SECONDS}" \
            -o Acquire::Retries="${APT_RETRIES}" download-file \
            "$uri" "/var/cache/apt/archives/$filename" "$digest" \
            "Checksum-FileSize:$size" &
        pids+=("$!")
        if [[ "${#pids[@]}" -ge "$APT_DOWNLOAD_CONCURRENCY" ]]; then
            for pid in "${pids[@]}"; do wait "$pid" || failed=1; done
            pids=()
            [[ "$failed" == 0 ]] || break
        fi
    done < "$manifest"
    # Reap every started worker even when a malformed row or download fails.
    if [[ "${#pids[@]}" -gt 0 ]]; then
        for pid in "${pids[@]}"; do wait "$pid" || failed=1; done
    fi
    [[ "$failed" == 0 ]] || return 1
    apt-get --no-download install -y --no-install-recommends "$@"
}
export -f materialize_archives
timeout --signal=TERM --kill-after=5s "${APT_INSTALL_TIMEOUT_SECONDS}s" \
    bash -c 'materialize_archives "$@"' host-apt \
    "${native_packages[@]}" "${cross_packages[@]}" \
    curl wget ca-certificates git sqlite3 python3-venv
