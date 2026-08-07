#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
    echo "usage: publish-immutable-release-assets.sh <release-tag> <owned-files-dir>" >&2
    exit 2
fi

release_tag="$1"
owned_dir="$2"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
existing_dir="$(mktemp -d)"
verified_dir="$(mktemp -d)"
missing_file="$(mktemp)"
verified_missing_file="$(mktemp)"

cleanup() {
    rm -rf "$existing_dir" "$verified_dir"
    rm -f "$missing_file" "$verified_missing_file"
}
trap cleanup EXIT

download_existing() {
    local destination="$1"
    local asset_count
    asset_count="$(gh release view "$release_tag" --json assets --jq '.assets | length')"
    if [[ "$asset_count" -gt 0 ]]; then
        gh release download "$release_tag" --dir "$destination"
    fi
}

if ! gh release view "$release_tag" >/dev/null 2>&1; then
    if [[ -z "${CAPSEM_RELEASE_CREATE_TITLE:-}" ]] \
        || [[ -z "${CAPSEM_RELEASE_CREATE_NOTES_FILE:-}" ]]; then
        echo "release $release_tag does not exist and creation metadata is absent" >&2
        exit 1
    fi
    create_args=(
        --title "$CAPSEM_RELEASE_CREATE_TITLE"
        --notes-file "$CAPSEM_RELEASE_CREATE_NOTES_FILE"
    )
    if [[ -n "${CAPSEM_RELEASE_CREATE_TARGET:-}" ]]; then
        create_args+=(--target "$CAPSEM_RELEASE_CREATE_TARGET")
    fi
    gh release create "$release_tag" "${create_args[@]}"
fi

download_existing "$existing_dir"
python3 "$script_dir/verify-immutable-publication.py" \
    --expected "$owned_dir" \
    --actual "$existing_dir" \
    --resume-owned \
    --missing-output "$missing_file"

while IFS= read -r missing; do
    [[ -n "$missing" ]] || continue
    if [[ "$missing" == channel-source-*.json ]]; then
        continue
    fi
    gh release upload "$release_tag" "$owned_dir/$missing"
done < "$missing_file"

# The source manifest is authoritative. Publish it only after every immutable
# byte it can reference is already available, so an interrupted first
# publication remains safely resumable.
while IFS= read -r missing; do
    [[ -n "$missing" ]] || continue
    if [[ "$missing" != channel-source-*.json ]]; then
        continue
    fi
    gh release upload "$release_tag" "$owned_dir/$missing"
done < "$missing_file"

download_existing "$verified_dir"
python3 "$script_dir/verify-immutable-publication.py" \
    --expected "$owned_dir" \
    --actual "$verified_dir" \
    --resume-owned \
    --missing-output "$verified_missing_file"
if [[ -s "$verified_missing_file" ]]; then
    echo "immutable publication remains incomplete after upload:" >&2
    sed 's/^/  /' "$verified_missing_file" >&2
    exit 1
fi
