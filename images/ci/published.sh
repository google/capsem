#!/bin/sh
# Whether a registry reference is published: prints `true` or `false`.
#
# Only a registry's "not found" answers `false`. Any other failure -- an
# outage, a denied token, a typo in the registry -- fails the step, because
# reading it as "absent" would rebuild and overwrite, and reading it as
# "present" would skip a build that never happened.
# Usage: images/ci/published.sh <reference>
set -eu

reference="${1:?usage: images/ci/published.sh <reference>}"

if error="$(oras manifest fetch --descriptor "$reference" 2>&1 >/dev/null)"; then
    echo true
    exit 0
fi
case "$error" in
    *"not found"* | *NOT_FOUND* | *MANIFEST_UNKNOWN* | *NAME_UNKNOWN*)
        echo false
        ;;
    *)
        printf '%s\n' "$error" >&2
        exit 1
        ;;
esac
