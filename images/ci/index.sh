#!/bin/sh
# Create one official image's multi-architecture index from its platform
# records, and record the index for the catalog.
#
# Every architecture of an image either rebuilt or skipped, because each leg
# asks the same question of the same key. Mixed answers, or legs that disagree
# on the key, mean the records do not describe one build, and nothing is
# published from them.
# Usage: images/ci/index.sh <repository> <image> <records-dir> <index-record>
# Prints `built=`, `key=` and `digest=` lines for $GITHUB_OUTPUT.
set -eu

repository="${1:?usage: images/ci/index.sh <repository> <image> <records-dir> <index-record>}"
image="${2:?missing image}"
records="${3:?missing records directory}"
out="${4:?missing index record path}"

legs="$(jq -s --arg image "$image" \
    'map(select(.kind == "platform" and .image == $image))
     | if length == 0 then error("no platform records for \($image)") else . end' \
    "$records"/*.json)"
built="$(printf '%s' "$legs" | jq -r \
    'map(.built) | unique | if length == 1 then .[0] else error("some architectures rebuilt, some did not") end')"
key="$(printf '%s' "$legs" | jq -r \
    'map(.key) | unique | if length == 1 then .[0] else error("architectures disagree on the input key") end')"

digest=""
if [ "$built" = true ]; then
    # shellcheck disable=SC2046 # one argument per digest, and digests hold no spaces
    docker buildx imagetools create --tag "$repository:$key" \
        $(printf '%s' "$legs" | jq -r --arg repository "$repository" '.[] | "\($repository)@\(.digest)"')
    digest="$(oras manifest fetch --descriptor "$repository:$key" | jq -r .digest)"
fi

jq -n --arg image "$image" --arg key "$key" --argjson built "$built" --arg digest "$digest" \
    '{kind: "index", image: $image, key: $key, built: $built}
     + (if $built then {digest: $digest} else {} end)' > "$out"
printf 'built=%s\nkey=%s\ndigest=%s\n' "$built" "$key" "$digest"
