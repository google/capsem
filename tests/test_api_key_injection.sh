#!/bin/bash
# Integration test: verify AI provider API key injection into guest VM.
#
# Tests both the settings-based injection (user.toml -> BootConfig -> guest env)
# and the --env CLI override path. Requires a built+signed binary and VM assets.
#
# Usage:
#   just test-api-keys                # via justfile recipe
#   ./tests/test_api_key_injection.sh # standalone (needs CAPSEM_ASSETS_DIR)
#
# What this tests:
#   1. Settings path: ai.google.api_key in user.toml -> GEMINI_API_KEY in guest
#   2. Settings path: ai.anthropic.api_key (toggle off) -> key NOT injected
#   3. CLI --env path: GEMINI_API_KEY injected and visible in guest
#   4. Gemini CLI: connects to Google AI API and gets an auth error (proves network + config work)

set -euo pipefail

BINARY="${CAPSEM_BINARY:-target/debug/capsem}"
ASSETS="${CAPSEM_ASSETS_DIR:-assets}"
USER_TOML="$HOME/.capsem/user.toml"
BACKUP=""
PASS=0
FAIL=0
TESTS=0

cleanup() {
    # Restore original user.toml
    if [ -n "$BACKUP" ] && [ -f "$BACKUP" ]; then
        mv "$BACKUP" "$USER_TOML"
    elif [ -n "$BACKUP" ]; then
        rm -f "$USER_TOML"
    fi
}
trap cleanup EXIT

# Back up existing user.toml
if [ -f "$USER_TOML" ]; then
    BACKUP="$(mktemp)"
    cp "$USER_TOML" "$BACKUP"
else
    BACKUP="__none__"
    mkdir -p "$(dirname "$USER_TOML")"
fi

run_in_vm() {
    CAPSEM_ASSETS_DIR="$ASSETS" "$BINARY" "$@" 2>&1
}

assert_contains() {
    local label="$1" output="$2" expected="$3"
    TESTS=$((TESTS + 1))
    if echo "$output" | grep -qF "$expected"; then
        echo "  PASS: $label"
        PASS=$((PASS + 1))
    else
        echo "  FAIL: $label"
        echo "    expected to find: $expected"
        echo "    got: $(echo "$output" | head -5)"
        FAIL=$((FAIL + 1))
    fi
}

assert_not_contains() {
    local label="$1" output="$2" unexpected="$3"
    TESTS=$((TESTS + 1))
    if echo "$output" | grep -qF "$unexpected"; then
        echo "  FAIL: $label"
        echo "    should NOT contain: $unexpected"
        echo "    got: $(echo "$output" | head -5)"
        FAIL=$((FAIL + 1))
    else
        echo "  PASS: $label"
        PASS=$((PASS + 1))
    fi
}

# ---------------------------------------------------------------
# Test 1: Settings-based injection (Google AI enabled + key set)
# ---------------------------------------------------------------
echo "=== Test 1: Settings-based API key injection (Google AI) ==="

cat > "$USER_TOML" << 'TOML'
[settings]
"ai.google.api_key" = { value = "test-settings-google-key", modified = "2026-02-25T00:00:00Z" }
TOML

OUTPUT=$(run_in_vm 'echo "GEMINI=$GEMINI_API_KEY"')

assert_contains "GEMINI_API_KEY set from settings" "$OUTPUT" "GEMINI=test-settings-google-key"

# ---------------------------------------------------------------
# Test 2: Disabled toggle -> key NOT injected
# ---------------------------------------------------------------
echo ""
echo "=== Test 2: Disabled toggle blocks key injection (Anthropic) ==="

cat > "$USER_TOML" << 'TOML'
[settings]
"ai.anthropic.api_key" = { value = "test-anthropic-key", modified = "2026-02-25T00:00:00Z" }
TOML
# ai.anthropic.allow defaults to false, so key should not be injected

OUTPUT=$(run_in_vm 'echo "ANT=$ANTHROPIC_API_KEY"')

assert_not_contains "ANTHROPIC_API_KEY not set when toggle off" "$OUTPUT" "test-anthropic-key"

# ---------------------------------------------------------------
# Test 3: Enabled toggle + key -> key IS injected
# ---------------------------------------------------------------
echo ""
echo "=== Test 3: Enabled toggle allows key injection (Anthropic) ==="

cat > "$USER_TOML" << 'TOML'
[settings]
"ai.anthropic.allow" = { value = true, modified = "2026-02-25T00:00:00Z" }
"ai.anthropic.api_key" = { value = "test-anthropic-key-on", modified = "2026-02-25T00:00:00Z" }
TOML

OUTPUT=$(run_in_vm 'echo "ANT=$ANTHROPIC_API_KEY"')

assert_contains "ANTHROPIC_API_KEY set when toggle on" "$OUTPUT" "ANT=test-anthropic-key-on"

# ---------------------------------------------------------------
# Test 4: CLI --env overrides settings
# ---------------------------------------------------------------
echo ""
echo "=== Test 4: CLI --env override ==="

# Clear user.toml so only --env matters
cat > "$USER_TOML" << 'TOML'
[settings]
TOML

OUTPUT=$(run_in_vm --env GEMINI_API_KEY=cli-override-key 'echo "GEMINI=$GEMINI_API_KEY"')

assert_contains "GEMINI_API_KEY from --env" "$OUTPUT" "GEMINI=cli-override-key"

# ---------------------------------------------------------------
# Test 5: Gemini CLI sees the API key and tries to authenticate
# ---------------------------------------------------------------
echo ""
echo "=== Test 5: Gemini CLI authentication attempt ==="

cat > "$USER_TOML" << 'TOML'
[settings]
"ai.google.api_key" = { value = "fake-key-for-auth-test", modified = "2026-02-25T00:00:00Z" }
TOML

# Run gemini with a prompt. It should attempt to connect to googleapis.com,
# get through the MITM proxy (domain is allowed), and fail with an auth error
# (invalid key). This proves the full pipeline: settings -> env var -> CLI -> network.
OUTPUT=$(run_in_vm 'echo "test" | gemini -t "say ok" 2>&1 || true')

# Gemini should either show an auth error (API key invalid) or a network response.
# It should NOT show "GEMINI_API_KEY not set" or similar missing-key errors.
assert_not_contains "Gemini does not complain about missing key" "$OUTPUT" "API key not"
assert_not_contains "Gemini does not complain about missing key (alt)" "$OUTPUT" "api key required"
assert_not_contains "Gemini does not complain about missing key (alt2)" "$OUTPUT" "GEMINI_API_KEY"

# It should show some kind of auth/API error (since the key is fake)
# or any HTTP-level response from Google -- proves network connectivity works.
TESTS=$((TESTS + 1))
if echo "$OUTPUT" | grep -qiE "invalid|unauthorized|error|API|403|401|authentication|credential|denied|failed|key"; then
    echo "  PASS: Gemini attempted API call (got auth/API error as expected with fake key)"
    PASS=$((PASS + 1))
else
    echo "  FAIL: Gemini output did not show expected auth error"
    echo "    got: $(echo "$OUTPUT" | tail -10)"
    FAIL=$((FAIL + 1))
fi

# ---------------------------------------------------------------
# Summary
# ---------------------------------------------------------------
echo ""
echo "=== Results: $PASS/$TESTS passed, $FAIL failed ==="
if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
