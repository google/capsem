#!/usr/bin/env bash
# Re-exports the Apple .p12 certificate with legacy 3DES/SHA1 encryption.
# macOS `security import` rejects modern PBES2/AES-256-CBC p12 files created
# by OpenSSL 3.x. This converts to the legacy format macOS understands.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CERT_DIR="$(dirname "$SCRIPT_DIR")/private/apple-certificate"
P12="$CERT_DIR/capsem.p12"
B64="$CERT_DIR/capsem-b64.txt"
PASS_FILE="$CERT_DIR/p12-password.txt"
TMPDIR_WORK="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_WORK"' EXIT

PASSWORD="$(tr -d '\n' < "$PASS_FILE")"

echo "==> Checking current format..."
FMT=$(openssl pkcs12 -in "$P12" -info -nokeys -nocerts -passin "pass:$PASSWORD" 2>&1 \
    | grep -o 'PBES2\|pbeWithSHA1And3-KeyTripleDES-CBC' | head -1)

if [[ "$FMT" != "PBES2" ]]; then
    echo "    Already legacy format ($FMT). Nothing to do."
    exit 0
fi

echo "    Current: $FMT (modern, macOS-incompatible)"
echo "==> Re-exporting with 3DES/SHA1..."

openssl pkcs12 -in "$P12" -passin "pass:$PASSWORD" -nodes -out "$TMPDIR_WORK/combined.pem" 2>/dev/null
openssl pkcs12 -export -in "$TMPDIR_WORK/combined.pem" -out "$TMPDIR_WORK/legacy.p12" \
    -passout "pass:$PASSWORD" -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1

cp "$TMPDIR_WORK/legacy.p12" "$P12"
base64 -i "$P12" -o "$B64"

echo "    Updated: $P12"
echo "    Updated: $B64"
echo ""
echo "==> Verify with: scripts/preflight.sh"
echo "==> Upload with: gh secret set APPLE_CERTIFICATE < $B64"
