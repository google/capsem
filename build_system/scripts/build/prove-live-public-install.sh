#!/usr/bin/env bash
set -euo pipefail

if (($# != 3)); then
  echo "usage: $0 <manifest-path> <manifest-url> <channel>" >&2
  exit 2
fi

manifest_path=$1
manifest_url=$2
channel=$3
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/../../.." && pwd)
capsem_home=${CAPSEM_HOME:-"$HOME/.capsem"}
capsem="$capsem_home/bin/capsem"
expected_version=$(uv run --project build_system --frozen python "$repo_root/scripts/release-package-contract.py" selected-version \
  --manifest "$manifest_path" --platform linux --architecture amd64)

curl -fsSL https://capsem.org/install.sh | CAPSEM_CHANNEL="$channel" sh
test -x "$capsem"
test -x /usr/bin/capsem-app
"$capsem" --version | grep -F "$expected_version"
dpkg-query -W -f='${Version}' capsem | grep -Fx "$expected_version"
for bin in capsem capsem-admin capsem-gateway capsem-mcp capsem-mcp-aggregator capsem-mcp-builtin capsem-process capsem-service capsem-tray capsem-tui capsem-mock-server capsem-bench-rs; do
  test -x "$capsem_home/bin/$bin"
  "$capsem_home/bin/$bin" --version | grep -F "$expected_version"
done
grep -F "$manifest_url" "$capsem_home/assets/manifest-metadata.json"
"$capsem" status | tee /tmp/capsem-live-status.txt
grep -F "Installed: true" /tmp/capsem-live-status.txt
grep -F "Running:   true" /tmp/capsem-live-status.txt
grep -F "Service:   ok" /tmp/capsem-live-status.txt
grep -F "Gateway:   ok" /tmp/capsem-live-status.txt
python3 "$repo_root/scripts/verify-installed-release.py" \
  --capsem "$capsem" \
  --manifest-url "$manifest_url" \
  --channel "$channel" \
  --package-version "$expected_version"
python3 "$repo_root/scripts/prove-installed-shell.py" \
  --capsem "$capsem" \
  --marker CAPSEM_LIVE_PUBLIC_INSTALL_SHELL_OK \
  --session-name release-live-public-shell-x86_64 \
  --timeout 300
