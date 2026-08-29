#!/usr/bin/env bash
# Apple signing and notarization preflight checks.

# --------------------------------------------------------------------------
# Check: Apple certificate can be imported into a macOS keychain
# macOS `security import` only supports legacy PKCS12 (3DES/SHA1).
# OpenSSL 3.x creates PBES2/AES-256-CBC by default, which Keychain rejects
# with a misleading "wrong password" error. Re-export with:
#   openssl pkcs12 -in cert.p12 -passin pass:PWD -nodes -out combined.pem
#   openssl pkcs12 -export -in combined.pem -out cert.p12 -passout pass:PWD \
#     -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1
# --------------------------------------------------------------------------
check_apple_certificate() {
    echo ""
    echo "== Apple Certificate =="

    local cert_dir="$ROOT_DIR/private/apple-certificate"
    local p12="$cert_dir/capsem.p12"
    local pass_file="$cert_dir/p12-password.txt"

    if [[ ! -f "$p12" ]]; then
        fail "capsem.p12 not found at $p12"
        return
    fi
    pass "capsem.p12 exists"

    local password
    password="$(tr -d '\n' < "$pass_file")"

    # Check encryption format
    local fmt
    fmt=$(openssl pkcs12 -in "$p12" -info -nokeys -nocerts -passin "pass:$password" 2>&1 \
        | grep -o 'PBES2\|pbeWithSHA1And3-KeyTripleDES-CBC' | head -1)

    if [[ "$fmt" == "PBES2" ]]; then
        fail "p12 uses modern PBES2/AES encryption (macOS incompatible) -- run: build_system/packaging/macos/fix_p12_legacy.sh"
        return
    fi
    pass "p12 uses legacy 3DES encryption (macOS-compatible)"

    # Try actual keychain import
    local keychain="preflight-$$.keychain"
    security create-keychain -p "" "$keychain" 2>/dev/null

    if ! security import "$p12" -k "$keychain" -P "$password" -T /usr/bin/codesign >/dev/null 2>&1; then
        security delete-keychain "$keychain" 2>/dev/null || true
        fail "keychain import failed"
        return
    fi
    pass "keychain import succeeded"

    security set-key-partition-list -S apple-tool:,apple: -k "" "$keychain" >/dev/null 2>&1

    local identity
    identity=$(security find-identity -v -p codesigning "$keychain" 2>/dev/null | grep "Developer ID" || true)
    security delete-keychain "$keychain" 2>/dev/null || true

    if [[ -z "$identity" ]]; then
        fail "no Developer ID signing identity found"
        return
    fi
    pass "signing identity: $(echo "$identity" | sed 's/.*"\(.*\)"/\1/')"
}

# --------------------------------------------------------------------------
# Check: base64-encoded certificate matches the p12 on disk
# --------------------------------------------------------------------------
check_b64_matches_p12() {
    echo ""
    echo "== Base64 Sync =="

    local cert_dir="$ROOT_DIR/private/apple-certificate"
    local p12="$cert_dir/capsem.p12"
    local b64="$cert_dir/capsem-b64.txt"

    if [[ ! -f "$b64" ]]; then
        fail "capsem-b64.txt not found"
        return
    fi

    local disk_b64
    disk_b64="$(base64 -i "$p12")"
    local file_b64
    file_b64="$(tr -d '\n\r ' < "$b64")"

    if [[ "$disk_b64" != "$file_b64" ]]; then
        fail "capsem-b64.txt does not match capsem.p12 -- regenerate with: base64 -i capsem.p12 -o capsem-b64.txt"
        return
    fi
    pass "capsem-b64.txt matches capsem.p12"
}

# --------------------------------------------------------------------------
# Check: Apple notarization credentials are present and work
# --------------------------------------------------------------------------
check_notarization() {
    echo ""
    echo "== Notarization =="

    local cert_dir="$ROOT_DIR/private/apple-certificate"
    local p8="$cert_dir/capsem.p8"
    local info="$cert_dir/api-key-info.txt"

    if [[ ! -f "$p8" ]]; then
        fail ".p8 key not found at $p8"
        return
    fi
    pass ".p8 key file exists"

    if [[ ! -f "$info" ]]; then
        fail "api-key-info.txt not found at $info"
        return
    fi

    local api_key api_issuer
    api_key=$(grep '^APPLE_API_KEY=' "$info" | head -1 | cut -d= -f2)
    api_issuer=$(grep '^APPLE_API_ISSUER=' "$info" | head -1 | cut -d= -f2)

    if [[ -z "$api_key" ]]; then
        fail "API Key ID not found in api-key-info.txt"
        return
    fi
    pass "API Key ID: $api_key"

    if [[ -z "$api_issuer" ]]; then
        fail "API Issuer ID not found in api-key-info.txt"
        return
    fi
    pass "API Issuer ID: $api_issuer"

    if ! command -v xcrun >/dev/null 2>&1; then
        fail "xcrun not found"
        return
    fi

    if ! xcrun notarytool --help >/dev/null 2>&1; then
        fail "xcrun notarytool not available"
        return
    fi
    pass "xcrun notarytool available"

    # Live check: verify credentials work against Apple's API (fast, no upload)
    if xcrun notarytool history \
        --key "$p8" \
        --key-id "$api_key" \
        --issuer "$api_issuer" \
        >/dev/null 2>&1; then
        pass "notarytool history succeeded (credentials valid)"
    else
        fail "notarytool history failed -- check .p8 key, Key ID, and Issuer ID"
    fi
}
