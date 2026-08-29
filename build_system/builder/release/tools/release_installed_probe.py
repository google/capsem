"""Release-owned installed shell probes used by glow-up qualification."""

from __future__ import annotations

import json
import shlex
import subprocess
import sys
import tempfile
from collections.abc import Callable
from pathlib import Path

HOST_BINARIES = (
    "capsem",
    "capsem-service",
    "capsem-process",
    "capsem-tui",
    "capsem-mcp",
    "capsem-mcp-aggregator",
    "capsem-mcp-builtin",
    "capsem-gateway",
    "capsem-tray",
    "capsem-admin",
    "capsem-mock-server",
    "capsem-bench-rs",
)


def packaged_manifest_metadata(deb: Path) -> dict[str, str]:
    """Return the future polling identity declared by an exact package.

    A package-ready run stages the exact publishable .deb rather than repacking
    it, so a fresh install records the channel identity built into that package.
    The hermetic server may redirect later channel transitions, but installation
    cannot rewrite this metadata or repoint the product at arbitrary HTTP input.
    """

    with tempfile.TemporaryDirectory() as extracted:
        subprocess.run(["dpkg-deb", "-x", str(deb), extracted], check=True)
        # Repacked and native packages use different prefixes, so locate the
        # one package-owned metadata file instead of assuming either layout.
        found = sorted(Path(extracted).rglob("assets/manifest-metadata.json"))
        if len(found) != 1:
            root = Path(extracted)
            layout = sorted(
                str(path.relative_to(root)) for path in root.rglob("*capsem*") if path.is_dir()
            )[:10]
            raise SystemExit(
                f"package must declare exactly one manifest metadata, found "
                f"{len(found)} in {deb}; capsem directories present: {layout or 'none'}"
            )
        packaged = json.loads(found[0].read_text(encoding="utf-8"))
    url = packaged.get("manifest_url")
    channel = packaged.get("channel")
    if not isinstance(url, str) or not isinstance(channel, str):
        raise SystemExit(f"package manifest metadata declares no channel identity: {deb}")
    return {"manifest_url": url, "channel": channel}


def clear_accelerated_automatic_update_polling(
    run: Callable[[list[str]], None],
) -> None:
    """Keep pairing-only systemd manager state out of the next module."""

    run(
        [
            "systemctl",
            "--user",
            "unset-environment",
            "CAPSEM_AUTOMATIC_UPDATE_INITIAL_DELAY_SECS",
            "CAPSEM_AUTOMATIC_UPDATE_POLL_SECS",
        ]
    )


def exact_installed_probe_shell(evidence_dir: Path) -> str:
    return f"""CAPSEM_BIN="$HOME/.capsem/bin/capsem"
CAPSEM_HOME_DIR="$HOME/.capsem"
EVIDENCE_DIR={shlex.quote(str(evidence_dir))}
mkdir -p "$EVIDENCE_DIR"
wait_for_service() {{
  for attempt in $(seq 1 90); do
    if CAPSEM_HOME="$CAPSEM_HOME_DIR" CAPSEM_RUN_DIR="$CAPSEM_HOME_DIR/run" \
      "$CAPSEM_BIN" status > "$EVIDENCE_DIR/service-status.txt" 2>&1 \
      && grep -Fq "Installed: true" "$EVIDENCE_DIR/service-status.txt" \
      && grep -Fq "Running:   true" "$EVIDENCE_DIR/service-status.txt" \
      && grep -Fq "Service:   ok" "$EVIDENCE_DIR/service-status.txt" \
      && grep -Fq "Gateway:   ok" "$EVIDENCE_DIR/service-status.txt"; then
      return 0
    fi
    sleep 2
  done
  cat "$EVIDENCE_DIR/service-status.txt" >&2 || true
  systemctl --user status capsem.service --no-pager -l >&2 || true
  journalctl --user-unit capsem.service --no-pager -n 200 >&2 || true
  return 1
}}
wait_for_profile_assets() {{
  profile="$1"
  output="$2"
  for attempt in $(seq 1 180); do
    if CAPSEM_HOME="$CAPSEM_HOME_DIR" CAPSEM_RUN_DIR="$CAPSEM_HOME_DIR/run" \
      "$CAPSEM_BIN" assets status --profile "$profile" --json > "$output" \
      && python3 - "$output" <<'PY'
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
  cat "$output" >&2 || true
  systemctl --user status capsem.service --no-pager -l >&2 || true
  journalctl --user-unit capsem.service --no-pager -n 200 >&2 || true
  return 1
}}
check_binary_versions() {{
  expected="$1"
  for binary in {" ".join(HOST_BINARIES)}; do
    test -x "$CAPSEM_HOME_DIR/bin/$binary"
    if [ "$binary" = capsem ]; then
      "$CAPSEM_HOME_DIR/bin/$binary" version
    else
      "$CAPSEM_HOME_DIR/bin/$binary" --version
    fi > "$EVIDENCE_DIR/$binary.version" 2>&1
    grep -F "$expected" "$EVIDENCE_DIR/$binary.version"
  done
}}
probe_installed_transition() {{
  label="$1"
  manifest_url="$2"
  channel="$3"
  package_version="$4"
  artifact="$5"
  platform="$6"
  architecture="$7"
  metadata_manifest_url="${{8:-$manifest_url}}"
  wait_for_service
  check_binary_versions "$package_version"
  dpkg-query -W -f='${{Version}}' capsem | grep -Fx "$package_version"
  {shlex.quote(sys.executable)} scripts/verify-installed-release.py \
    --capsem "$CAPSEM_BIN" \
    --capsem-home "$CAPSEM_HOME_DIR" \
    --manifest-url "$manifest_url" \
    --metadata-manifest-url "$metadata_manifest_url" \
    --channel "$channel" \
    --package-version "$package_version" \
    --artifact "$artifact" \
    --platform "$platform" \
    --architecture "$architecture" \
    --evidence-out "$EVIDENCE_DIR/$label-installed.json"
  doctor_log="$EVIDENCE_DIR/$label-doctor.log"
  failed_process_logs="$EVIDENCE_DIR/$label-failed-process-logs.txt"
  service_evidence="$EVIDENCE_DIR/$label-service-logs.txt"
  if ! CAPSEM_HOME="$CAPSEM_HOME_DIR" CAPSEM_RUN_DIR="$CAPSEM_HOME_DIR/run" \
    "$CAPSEM_BIN" doctor > "$doctor_log" 2>&1; then
    : > "$failed_process_logs"
    while IFS= read -r process_log; do
      printf '\n===== %s =====\n' "$process_log" >> "$failed_process_logs"
      tail -n 200 "$process_log" >> "$failed_process_logs" 2>&1 || true
    done < <(
      find "$CAPSEM_HOME_DIR/run/sessions" -type f -name process.log \
        -path "*-failed-*" -print 2>> "$failed_process_logs" || true
    )
    : > "$service_evidence"
    while IFS= read -r service_log; do
      printf '\n===== %s =====\n' "$service_log" | tee -a "$service_evidence" >&2
      tail -n 200 "$service_log" | tee -a "$service_evidence" >&2 || true
    done < <(service_logs)
    cat "$doctor_log" >&2
    cat "$failed_process_logs" >&2
    systemctl --user status capsem.service --no-pager -l >&2 || true
    journalctl --user-unit capsem.service --no-pager -n 200 >&2 || true
    return 1
  fi
  printf '%s\n' '{{"schema":"capsem.installed_doctor.v1","passed":true}}' \
    > "$EVIDENCE_DIR/$label-doctor.json"
  {shlex.quote(sys.executable)} build_system/scripts/build/run-installed-winterfell.py \
    --bin-dir "$CAPSEM_HOME_DIR/bin" \
    --assets-dir "$CAPSEM_HOME_DIR/assets" \
    --profiles-dir "$CAPSEM_HOME_DIR/profiles" \
    --evidence-out "$EVIDENCE_DIR/$label-winterfell.json"
}}
observe_update_transition() {{
  kind="$1"
  result="$2"
  source="$3"
  candidate_manifest_sha="$4"
  after_line="$5"
  evidence_out="$6"
  previous_manifest_sha="${{7:-}}"
  command=(
    {shlex.quote(sys.executable)} scripts/release_transition.py
    --audit-log "$CAPSEM_HOME_DIR/logs/update.log"
    --after-line "$after_line"
    --kind "$kind"
    --result "$result"
    --source "$source"
    --candidate-manifest-sha256 "$candidate_manifest_sha"
    --timeout-seconds 180
    --evidence-out "$evidence_out"
  )
  if [ -n "$previous_manifest_sha" ]; then
    command+=(--previous-manifest-sha256 "$previous_manifest_sha")
  fi
  "${{command[@]}}" || {{
    dump_update_diagnostics "$kind $result"
    return 1
  }}
}}
installed_profile_tree_digest() {{
  find "$CAPSEM_HOME_DIR/profiles" -type f -print0 \
    | sort -z \
    | xargs -0 sha256sum \
    | sha256sum \
    | cut -d' ' -f1
}}
# A rejection proof has two ways to fail and they mean opposite things: the
# service saw the tampered manifest and installed it anyway, or it never saw
# it. Attempt 37 waited three minutes on the second and reported the first.
# Checking the wire first makes the timeout say which.
assert_manifest_served() {{
  url="$1"
  expected="$2"
  what="$3"
  actual=$(curl -fsSL "$url" | sha256sum | cut -d' ' -f1) || {{
    echo "FATAL: $what: cannot fetch $url" >&2
    return 1
  }}
  if [ "$actual" != "$expected" ]; then
    echo "FATAL: $what was never served." >&2
    echo "  $url returns sha256=$actual" >&2
    echo "  the staged candidate is    sha256=$expected" >&2
    echo "  So this is a staging failure, not a refusal to reject." >&2
    return 1
  fi
}}
service_logs() {{
  # `service.log` and `service.<date>.log`, never `services.log` -- the same
  # membership test `capsem_core::telemetry::log_stream_files` applies. A bare
  # `service*.log` also matches a neighbouring stream, which would let an
  # unrelated file satisfy a rejection proof.
  ls -1t "$CAPSEM_HOME_DIR/run/service.log" \
         "$CAPSEM_HOME_DIR/run"/service.*.log 2>/dev/null || true
}}
# Everything a reader needs to tell the failure modes apart, printed once on
# the way out: whether the polling loop started at all and on what schedule,
# what it decided each cycle, whether systemd thinks the unit is up, and the
# service's own log. Not knowing which of these was true is what made the
# last failure unreadable.
dump_update_diagnostics() {{
  what="$1"
  echo "=== $what did not happen; diagnostics follow ===" >&2
  echo "--- automatic update loop decisions ---" >&2
  # shellcheck disable=SC2046
  grep -F "automatic release" $(service_logs) 2>&1 | tail -40 >&2 || true
  echo "--- service log tail ---" >&2
  echo "service logs found: $(service_logs | tr '\n' ' ')" >&2
  # shellcheck disable=SC2046
  tail -80 $(service_logs) >&2 2>&1 || echo "no service log under $CAPSEM_HOME_DIR/run" >&2
  echo "--- update log ---" >&2
  tail -40 "$CAPSEM_HOME_DIR/logs/update.log" >&2 2>&1 || true
  echo "--- systemd unit ---" >&2
  systemctl --user status capsem.service --no-pager -l >&2 2>&1 || true
  echo "--- unit environment ---" >&2
  systemctl --user show-environment >&2 2>&1 || true
  echo "--- journal (systemd's own view) ---" >&2
  journalctl --user-unit capsem.service --no-pager -n 200 -o cat >&2 2>&1 || true
}}
"""
