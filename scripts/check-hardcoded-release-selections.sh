#!/usr/bin/env bash
set -euo pipefail

ROOT="${CAPSEM_GUARD_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

# These are current and planned profile names. They may appear in profile
# definitions, fixtures, and explicit default declarations, but never as a
# literal selection in a user-facing request or release package rail.
PROFILE_TERMS='(code|co-work|cowork|terminal|termional|gui)'
failed=0

reject_matches() {
    local label="$1"
    local pattern="$2"
    shift 2
    local matches
    if matches=$(rg -n -U --color never -- "$pattern" "$@" 2>/dev/null); then
        echo "ERROR: $label" >&2
        printf '%s\n' "$matches" >&2
        failed=1
    fi
}

reject_matches \
    "user-facing session request hardcodes a named profile" \
    "profile_id[[:space:]]*:[[:space:]]*['\"]${PROFILE_TERMS}['\"]" \
    frontend/src/lib/components crates/capsem-tray/src

reject_matches \
    "profile picker fabricates a named profile instead of using the installed catalog" \
    "(profileId[[:space:]]*=[^\n]*['\"]${PROFILE_TERMS}['\"]|<option[^>]*value=['\"]${PROFILE_TERMS}['\"])" \
    frontend/src/lib/components

reject_matches \
    "MCP request bypasses its explicit profile parameter" \
    "['\"]profile_id['\"][[:space:]]*:[[:space:]]*DEFAULT_PROFILE_ID" \
    crates/capsem-mcp/src/main.rs

reject_matches \
    "profile-scoped MCP route silently uses the default profile" \
    "['\"]/profiles/\\{\\}/mcp[^;]{0,240}DEFAULT_PROFILE_ID" \
    crates/capsem/src/main.rs crates/capsem-mcp/src/main.rs

configured_profiles=$(rg --files config/profiles \
    | rg '/profile\.toml$' \
    | sed -E 's#config/profiles/([^/]+)/profile\.toml#\1#' \
    | sort)
embedded_profiles=$(rg -o --no-filename 'config/profiles/[^/]+/profile\.toml' \
    crates/capsem-core/src/net/policy_config/profile_contract.rs \
    | sed -E 's#config/profiles/([^/]+)/profile\.toml#\1#' \
    | sort -u)
if [[ "$configured_profiles" != "$embedded_profiles" ]]; then
    echo "ERROR: builtin_profile_configs does not exactly mirror config/profiles" >&2
    echo "configured profiles:" >&2
    printf '%s\n' "$configured_profiles" >&2
    echo "embedded profiles:" >&2
    printf '%s\n' "$embedded_profiles" >&2
    failed=1
fi

reject_matches \
    "release packaging materializes one named profile instead of the catalog" \
    "--profile[[:space:]]+[^[:space:]]*${PROFILE_TERMS}" \
    .github/workflows/release.yaml

reject_matches \
    "workflow input silently defaults a profile or public release channel" \
    "(channel|asset_channel|profile):[[:space:]]*\n([^\n]*\n){0,8}[[:space:]]*default:[[:space:]]*(${PROFILE_TERMS}|stable|nightly)[[:space:]]*\n" \
    .github/workflows

reject_matches \
    "release qualification hardcodes stable/nightly instead of its channel input" \
    "CAPSEM_INSTALL_(MANIFEST_URL|CHANNEL):.*(stable|nightly)" \
    .github/workflows/release-qualification.yaml

reject_matches \
    "release workflow hardcodes a stable/nightly ASSET_MANIFEST_URL instead of an explicit channel input" \
    "ASSET_MANIFEST_URL:.*assets/(stable|nightly)/manifest\\.json" \
    .github/workflows

reject_matches \
    "reusable release deployment makes its channel optional" \
    "channel:[[:space:]]*\n([^\n]*\n){0,3}[[:space:]]*required:[[:space:]]*false" \
    .github/workflows/release-channel.yaml

reject_matches \
    "reusable release deployment silently substitutes stable for its channel input" \
    "inputs\\.channel[[:space:]]*\\|\\|[[:space:]]*['\"]stable['\"]" \
    .github/workflows/release-channel.yaml

reject_matches \
    "native postinstall silently falls back to a public channel" \
    "MANIFEST_SOURCE=['\"]https://release\\.capsem\\.org/assets/(stable|nightly)/manifest\\.json['\"]" \
    scripts/deb-postinst.sh scripts/pkg-scripts/postinstall

reject_matches \
    "native postinstall bypasses installed manifest-metadata provenance" \
    "CAPSEM_RELEASE_(MANIFEST|HEALTH)_URL" \
    scripts/deb-postinst.sh scripts/pkg-scripts/postinstall

reject_matches \
    "release qualification bypasses installed manifest-metadata provenance" \
    "CAPSEM_RELEASE_(MANIFEST|HEALTH)_URL=" \
    .github/workflows/release-qualification.yaml

reject_matches \
    "installed update test bypasses manifest-metadata provenance" \
    "['\"]CAPSEM_RELEASE_(MANIFEST|HEALTH)_URL['\"][[:space:]]*:" \
    tests/capsem-install

reject_matches \
    "legacy split manifest/update sidecar was reintroduced" \
    "manifest-origin\\.json|update-check\\.json" \
    scripts/build-pkg.sh scripts/repack-deb.sh scripts/deb-postinst.sh \
    scripts/pkg-scripts/postinstall crates/capsem/src/update.rs \
    crates/capsem-service/src/main.rs

reject_matches \
    "installed update flow silently substitutes the stable manifest when source metadata is absent" \
    "unwrap_or(_else)?\\([^\n]*DEFAULT_RELEASE_MANIFEST_URL" \
    crates/capsem/src/update.rs

qualification_calls=$(rg -n --color never 'check-release-qualification\.py' \
    justfile .github/workflows/release.yaml || true)
missing_channel=$(printf '%s\n' "$qualification_calls" | rg -v -- '--channel' || true)
if [[ -n "$missing_channel" ]]; then
    echo "ERROR: release qualification check is not bound to an explicit channel" >&2
    printf '%s\n' "$missing_channel" >&2
    failed=1
fi

if [[ "$failed" -ne 0 ]]; then
    exit 1
fi

echo "Hardcoded profile/channel selection guard passed."
