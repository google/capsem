#!/usr/bin/env python3
"""A sandboxed VM prompts a local Ollama/Gemma model through capsem.

The guest holds no model weights and has no open internet. Its only route to
a completion is capsem's egress: the guest redirects any connection to
``:11434`` into its net-proxy (see guest/artifacts/capsem-init), the host
proxy forwards it to the Ollama daemon on the host, and the ``ai_ollama_*``
profile rules admit and record the call. So a one-line prompt from inside the
VM comes back answered by the host's Gemma, and the security ledger has a row
for it.

Point it at whatever model `ollama list` shows on the host (default gemma4).

Usage (build first, then bound the run so no VM leaks):
    uv run --project build_system --frozen capsem-gate sign
    python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 900 \
        -- uv run --project build_system --frozen python tests/manual/vm_ollama.py

Env: CAPSEM_GYM_MODEL (default "gemma4"), CAPSEM_GYM_PROMPT.
"""

from __future__ import annotations

import contextlib
import os
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

# Only sys.path calls may sit above these first-party imports, or E402 fires;
# the imports cannot move up, they need the path set first.
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tests"))
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from helpers.constants import BIN_DIR, CODE_PROFILE_ID
from helpers.service import ServiceInstance

from tests.fixtures.oci.registry import registry

os.environ.setdefault("CAPSEM_TRAY_HEADLESS", "1")

MODEL = os.environ.get("CAPSEM_GYM_MODEL", "gemma4")
PROMPT = os.environ.get(
    "CAPSEM_GYM_PROMPT",
    "You are running inside a sandboxed VM. In one sentence, say hello and name the capital of France.",
)


def boot(service, tmp_path, reference, certificate, name, *options):
    """Create one named Redis container VM; return its /vms/list row once Redis is up.

    `capsem create --image` starts the workload detached, the way the
    kingslanding suite does (`tests/ironbank/kingslanding/test_run.py`), so the
    container's output lands on the guest console rather than on a pipe.
    `capsem run` used to take `-n NAME REFERENCE` and stay attached; it now
    destroys its VM on exit and takes the image as `--image`, which is why
    every manual scenario booting through the old spelling stopped at argv
    parsing. `options` go before `--image`, which consumes everything after it.
    """
    command = [
        str(BIN_DIR / "capsem"), "--uds-path", str(service.uds_path),
        "create", "--profile", CODE_PROFILE_ID, "--registry-ca", str(certificate),
        "-n", name, *options, "--image", reference,
    ]
    env = {
        **os.environ,
        "CAPSEM_HOME": str(service.home_dir),
        "CAPSEM_RUN_DIR": str(service.tmp_dir),
        "CAPSEM_PROFILES_DIR": str(service.profiles_dir),
    }
    result = subprocess.run(command, env=env, capture_output=True, timeout=240, check=False)
    (tmp_path / f"{name}.stderr").write_bytes(result.stderr)
    if result.returncode != 0:
        raise RuntimeError(f"{name} create failed:\n{result.stderr.decode(errors='replace')}")
    rows = [r for r in service.client().get("/vms/list")["sandboxes"] if r.get("name") == name]
    if len(rows) != 1:
        raise RuntimeError(f"expected one {name} VM, saw {rows}")
    deadline = time.time() + 180
    while time.time() < deadline:
        serial = service.client().get(f"/vms/{rows[0]['id']}/logs").get("serial_logs") or ""
        if "Ready to accept connections tcp" in serial:
            return rows[0]
        time.sleep(0.5)
    raise RuntimeError(f"{name} never became ready:\n{serial[-2000:]}")


def guest(service, vm_id, shell, timeout=180):
    return service.client().post(
        f"/vms/{vm_id}/exec", {"command": shell, "timeout_secs": timeout}, timeout=timeout + 5
    )


# Runs in the guest's main namespace (not the container's), where the :11434
# redirect to the net-proxy applies. urllib is in the guest's Python.
GENERATE = """python3 - <<'PY'
import json, urllib.request
body = json.dumps({{"model": {model!r}, "prompt": {prompt!r}, "stream": False}}).encode()
req = urllib.request.Request(
    "http://localhost:11434/api/generate", data=body,
    headers={{"Content-Type": "application/json"}},
)
data = json.loads(urllib.request.urlopen(req, timeout=150).read().decode())
print("MODEL=" + str(data.get("model")))
print("ANSWER=" + " ".join(str(data.get("response", "")).split()))
PY"""


def main() -> int:
    service = ServiceInstance()
    service.start()
    client = service.client()
    booted = []
    try:
        with registry(service.tmp_dir) as (reference, certificate, _requests):
            print(f"\n== boot a sandboxed VM (profile {CODE_PROFILE_ID}) ==")
            box = boot(service, service.tmp_dir, reference, certificate, "box")
            booted.append(box["id"])
            print(f"  box {box['id']}  (no model weights, no open internet)")

            print(f"\n== the VM prompts {MODEL} through capsem's egress ==")
            print(f"  prompt: {PROMPT}")
            response = guest(service, box["id"], GENERATE.format(model=MODEL, prompt=PROMPT), timeout=200)
            out = response.get("stdout", "")
            if response.get("exit_code") != 0:
                print(f"  [FAIL] the call did not complete (exit {response.get('exit_code')})")
                print("  stdout:", out.strip()[:500])
                print("  stderr:", response.get("stderr", "").strip()[:500])
                return 1
            answered_model = next((line[6:] for line in out.splitlines() if line.startswith("MODEL=")), "")
            answer = next((line[7:] for line in out.splitlines() if line.startswith("ANSWER=")), "")
            print(f"  [PASS] {answered_model} answered from inside the VM")
            print(f"  gemma: {answer}")

            print("\n== capsem recorded the model egress in the session ledger ==")
            recorded = wait_for_ledger(service.tmp_dir, box["id"])
            if recorded:
                print(f"  [PASS] {recorded}")
            else:
                print("  [FAIL] no model_calls or :11434 net_events row appeared")

            ok = bool(answer) and bool(recorded)
            print(f"\n== {'GEMMA ANSWERED THE SANDBOXED VM' if ok else 'FAILURES ABOVE'} ==")
            return 0 if ok else 1
    finally:
        for vm_id in booted:
            with contextlib.suppress(Exception):
                client.delete(f"/vms/{vm_id}/delete")
        service.stop()


def wait_for_ledger(run_dir: Path, vm_id: str, timeout: float = 25.0) -> str:
    """Poll the VM's own session DB until the model call is recorded.

    A guest's model call reaches the session ledger asynchronously (the writer
    batches and flushes), so this polls rather than reads once. Prefers the
    typed `model_calls` row (protocol ollama); falls back to the raw
    `net_events` row on the Ollama port. Read-only, matching the ledger tests.
    """
    db = run_dir / "persistent" / vm_id / "session.db"
    deadline = time.time() + timeout
    while time.time() < deadline:
        if db.exists():
            with contextlib.suppress(sqlite3.Error), closing_ro(db) as conn:
                row = conn.execute(
                    "SELECT protocol, model, path, status_code FROM model_calls "
                    "WHERE protocol = 'ollama' ORDER BY id DESC LIMIT 1"
                ).fetchone()
                if row:
                    return f"model_calls row: protocol={row[0]} model={row[1]} path={row[2]} status={row[3]}"
                net = conn.execute(
                    "SELECT domain, port, method, path, decision FROM net_events "
                    "WHERE port = 11434 ORDER BY id DESC LIMIT 1"
                ).fetchone()
                if net:
                    return f"net_events row: {net[0]}:{net[1]} {net[2]} {net[3]} decision={net[4]}"
        time.sleep(1)
    return ""


def closing_ro(db: Path):
    """A read-only sqlite connection context manager (never locks the writer)."""
    return contextlib.closing(sqlite3.connect(f"file:{db}?mode=ro", uri=True, timeout=2))


if __name__ == "__main__":
    raise SystemExit(main())
