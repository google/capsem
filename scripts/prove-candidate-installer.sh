#!/usr/bin/env bash
set -euo pipefail

if (($# != 3)); then
  echo "usage: $0 <candidate-dist> <channel> <expected-version>" >&2
  exit 2
fi

candidate_dist=$1
channel=$2
expected_version=$3
manifest="$candidate_dist/assets/$channel/manifest.json"
test -s "$manifest"

python3 -m http.server 8765 --directory "$candidate_dist" >/tmp/candidate-http.log 2>&1 &
server_pid=$!
trap 'kill "$server_pid" 2>/dev/null || true; cat /tmp/candidate-http.log' EXIT
for _ in $(seq 1 20); do
  if curl -fsSL "http://127.0.0.1:8765/assets/$channel/manifest.json" >/dev/null; then
    break
  fi
  sleep 0.25
done
candidate_manifest="http://127.0.0.1:8765/assets/$channel/manifest.json"
docker run --rm --network host \
  -e CAPSEM_MANIFEST_URL="$candidate_manifest" \
  -e CAPSEM_CHANNEL="$channel" \
  -e EXPECTED_VERSION="$expected_version" \
  ubuntu:24.04 bash -lc '
    set -euo pipefail
    export DEBIAN_FRONTEND=noninteractive
    apt-get update
    apt-get install -y ca-certificates curl sudo
    useradd -m -s /bin/bash capsemtest
    printf "%s\n" "capsemtest ALL=(ALL) NOPASSWD:ALL" >/etc/sudoers.d/capsemtest
    chmod 0440 /etc/sudoers.d/capsemtest
    su capsemtest -c "curl -fsSL https://capsem.org/install.sh | CAPSEM_MANIFEST_URL=$CAPSEM_MANIFEST_URL CAPSEM_CHANNEL=$CAPSEM_CHANNEL sh"
    dpkg-query -W -f="\${Version}" capsem | grep -Fx "$EXPECTED_VERSION"
    su capsemtest -c "\$HOME/.capsem/bin/capsem --version" | grep -F "$EXPECTED_VERSION"
  '
