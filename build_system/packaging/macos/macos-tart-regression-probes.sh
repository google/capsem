#!/bin/bash
# Installed-package regression probes sourced by macos_tart_guest.sh.

ASSET_HYDRATION_EVIDENCE="$SHARE/asset-hydration-evidence.json"
STALE_HELPER_EVIDENCE="$SHARE/stale-helper-evidence.json"

verify_binary_cohort() {
    local binary path
    for binary in "${BINARIES[@]}"; do
        path="$CAPSEM_BIN_DIR/$binary"
        test -x "$path"
        "$path" --version | grep -F "$VERSION"
        codesign --verify --strict "$path"
        codesign -d --verbose=4 "$path" 2>&1 | grep -F "Signature=adhoc"
    done
}

capsem_wait_for_service() {
    local attempt status
    for attempt in $(seq 1 90); do
        status=$("$CAPSEM" status 2>/dev/null || true)
        if grep -Fq "Service:   ok" <<<"$status" \
            && grep -Fq "Gateway:   ok" <<<"$status"
        then
            return 0
        fi
        sleep 2
    done
    printf '%s\n' "$status" >&2
    return 1
}

capsem_wait_for_profile_assets() {
    local profile="$1" output="$2" attempt
    for attempt in $(seq 1 180); do
        "$CAPSEM" assets status --profile "$profile" --json > "$output"
        if python3 - "$output" <<'PY'
import json
from pathlib import Path
import sys

status = json.loads(Path(sys.argv[1]).read_text())
raise SystemExit(0 if status.get("ready") and not status.get("downloading") else 1)
PY
        then
            return 0
        fi
        sleep 1
    done
    echo "ERROR: profile $profile assets did not settle after $attempt polls" >&2
    cat "$output" >&2
    return 1
}

capsem_finish_install_hydration() {
    capsem_wait_for_service
    grep -Fq "event=manifest_installed" "$CAPSEM_HOME/logs/install-latest.log"
    if grep -Fq "event=assets_hydrated" "$CAPSEM_HOME/logs/install-latest.log"; then
        echo "ERROR: package installer synchronously hydrated VM assets" >&2
        return 1
    fi
    capsem_wait_for_profile_assets code "$SHARE/code-assets-after-install.json"
    capsem_wait_for_profile_assets co-work "$SHARE/co-work-assets-after-install.json"
}

capsem_probe_asset_hydration() {
    local asset_path
    asset_path=$(python3 - "$SHARE/code-assets-after-install.json" <<'PY'
import json
from pathlib import Path
import sys

assets = json.loads(Path(sys.argv[1]).read_text())["assets"]
selected = next((asset for asset in assets if "rootfs" not in asset["name"]), assets[0])
print(selected["path"])
PY
)
    test -f "$asset_path"
    rm "$asset_path"
    "$CAPSEM" assets ensure --profile code --json > "$SHARE/code-assets-repair-start.json"
    python3 - "$SHARE/code-assets-repair-start.json" <<'PY'
import json
from pathlib import Path
import sys

status = json.loads(Path(sys.argv[1]).read_text())
if status.get("started") is not True or status.get("downloading") is not True:
    raise SystemExit(f"asset repair did not acknowledge background start: {status}")
if status.get("ready") is not False:
    raise SystemExit(f"asset repair claimed readiness before background completion: {status}")
PY
    capsem_wait_for_profile_assets code "$SHARE/code-assets-repair-complete.json"
    python3 - "$ASSET_HYDRATION_EVIDENCE" \
        "$SHARE/code-assets-repair-start.json" \
        "$SHARE/code-assets-repair-complete.json" <<'PY'
import json
from pathlib import Path
import sys

started = json.loads(Path(sys.argv[2]).read_text())
completed = json.loads(Path(sys.argv[3]).read_text())
Path(sys.argv[1]).write_text(
    json.dumps(
        {
            "schema": "capsem.asset_hydration_glowup.v1",
            "manifest_only_install": True,
            "started": started.get("started") is True,
            "downloading": started.get("downloading") is True,
            "completed_ready": completed.get("ready") is True,
        },
        indent=2,
        sort_keys=True,
    )
    + "\n"
)
PY
}

capsem_probe_stale_helper_replacement() {
    local old_service_pid new_service_pid
    old_service_pid=$(cat "$CAPSEM_HOME/run/service.pid")
    kill -0 "$old_service_pid"
    bash "$INSTALL_USER_REQUEST" write admin
    bash "$INSTALL_MANIFEST_REQUEST" write "$REMOTE_MANIFEST" "$MANIFEST_URL"
    sudo /usr/sbin/installer -pkg "$PKG" -target /
    clear_install_user_request
    capsem_wait_for_service
    new_service_pid=$(cat "$CAPSEM_HOME/run/service.pid")
    test "$new_service_pid" != "$old_service_pid"
    if kill -0 "$old_service_pid" 2>/dev/null; then
        echo "ERROR: old installed service survived package replacement" >&2
        return 1
    fi
    python3 - "$STALE_HELPER_EVIDENCE" "$old_service_pid" "$new_service_pid" <<'PY'
import json
from pathlib import Path
import sys

Path(sys.argv[1]).write_text(
    json.dumps(
        {
            "schema": "capsem.stale_helper_replacement.v1",
            "old_service_pid": int(sys.argv[2]),
            "new_service_pid": int(sys.argv[3]),
            "old_service_retired": True,
        },
        indent=2,
        sort_keys=True,
    )
    + "\n"
)
PY
}
