"""Archive an in-VM local MITM benchmark artifact.

This is intentionally gated by CAPSEM_RUN_MITM_LOCAL_BENCH=1 because it boots a
VM and needs the debug upstream URL to be routable through the Capsem network
path. When no explicit CAPSEM_BENCH_MITM_LOCAL_BASE_URL is supplied, the test
starts capsem-debug-upstream on host localhost and passes that URL to the guest.
"""

import json
import os
import re
import selectors
import shlex
import sqlite3
import subprocess
import time
import uuid
from pathlib import Path
from urllib.parse import urlsplit

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = [pytest.mark.serial, pytest.mark.benchmark]

PROJECT_ROOT = Path(__file__).parent.parent.parent
DEBUG_UPSTREAM_BINARY = PROJECT_ROOT / "target" / "debug" / "capsem-debug-upstream"
DEBUG_UPSTREAM_ADDR = "127.0.0.1:11434"


def _project_version():
    cargo = PROJECT_ROOT / "Cargo.toml"
    match = re.search(r'^version\s*=\s*"([^"]+)"', cargo.read_text(), re.MULTILINE)
    return match.group(1) if match else "unknown"


def _archive(data):
    version = _project_version()
    arch = "arm64" if os.uname().machine == "arm64" else "x86_64"
    out_dir = PROJECT_ROOT / "benchmarks" / "mitm-local"
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / f"data_{version}_{arch}.json"
    with open(out_path, "w") as handle:
        json.dump(data, handle, indent=2)
    print(f"mitm-local benchmark archived to {out_path}")
    return out_path


def _read_ready_json(proc, timeout_s=10):
    selector = selectors.DefaultSelector()
    selector.register(proc.stdout, selectors.EVENT_READ)
    deadline = time.monotonic() + timeout_s
    lines = []
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            raise RuntimeError(
                f"capsem-debug-upstream exited early with code {proc.returncode}: "
                f"{''.join(lines)}"
            )
        events = selector.select(timeout=0.2)
        for key, _ in events:
            line = key.fileobj.readline()
            if not line:
                continue
            lines.append(line)
            try:
                payload = json.loads(line)
            except json.JSONDecodeError:
                continue
            if payload.get("service") == "capsem-debug-upstream":
                return payload
    raise TimeoutError(
        "capsem-debug-upstream did not print ready JSON; "
        f"stdout={''.join(lines)!r}"
    )


def _start_debug_upstream():
    if not DEBUG_UPSTREAM_BINARY.exists():
        pytest.skip(
            f"{DEBUG_UPSTREAM_BINARY} not found; run `cargo build -p capsem-debug-upstream`"
        )
    proc = subprocess.Popen(
        [str(DEBUG_UPSTREAM_BINARY), "--addr", DEBUG_UPSTREAM_ADDR],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )
    try:
        ready = _read_ready_json(proc)
        return proc, ready
    except Exception:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
        raise


def _stop_process(proc):
    if proc is None:
        return
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()


def _assert_mitm_local_succeeded(data):
    assert "mitm_local" in data
    result = data["mitm_local"]
    total_requests = result["total_requests"]

    for row in result["scenarios"]:
        assert row["successful"] == total_requests, (
            f"{row['name']} should complete every request: {row}"
        )
        assert row["failed"] == 0, (
            f"{row['name']} should have no failed requests: {row}"
        )
        assert not row["errors"], (
            f"{row['name']} should have no transport errors: {row['errors']}"
        )

    for row in result["websocket"]:
        assert not row.get("skipped"), f"{row['name']} should not be skipped: {row}"
        assert not row.get("failed"), f"{row['name']} should not fail: {row}"
        assert row["frames"] > 0, f"{row['name']} should relay frames: {row}"


def _write_local_benchmark_policy(capsem_home, base_url):
    parsed = urlsplit(base_url)
    port = parsed.port or (443 if parsed.scheme == "https" else 80)
    capsem_home.mkdir(parents=True, exist_ok=True)
    (capsem_home / "user.toml").write_text(
        f"""
[settings."security.web.http_upstream_ports"]
value = [80, 11434, {port}]
modified = "2026-06-06T00:00:00Z"
""".lstrip()
    )


def _assert_session_db_contains_mitm_events(capsem_home, vm_name, total_requests):
    db_path = capsem_home / "sessions" / vm_name / "session.db"
    expected_paths = {
        "/tiny",
        "/bytes/1mb",
        "/gzip/1mb",
        "/sse/model",
        "/deny-target",
        "/credential/response",
        "/ws/echo",
        "/ws/close",
    }
    expected_count = total_requests * 6 + 2

    deadline = time.monotonic() + 5
    rows = []
    while time.monotonic() < deadline:
        if db_path.exists():
            conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
            try:
                rows = conn.execute(
                    """
                    SELECT path, status_code, decision
                    FROM net_events
                    WHERE domain = '127.0.0.1'
                    ORDER BY id
                    """
                ).fetchall()
            finally:
                conn.close()
            if len(rows) >= expected_count:
                break
        time.sleep(0.1)

    assert db_path.exists(), f"session.db not found at {db_path}"
    assert len(rows) >= expected_count, (
        f"expected at least {expected_count} local MITM net_events, got {len(rows)}: {rows}"
    )
    paths = {row[0] for row in rows}
    assert expected_paths.issubset(paths), (
        f"session.db missing benchmark paths: {expected_paths - paths}; rows={rows}"
    )
    assert any(row[1] == 101 for row in rows), (
        f"session.db should include WebSocket 101 upgrade events: {rows}"
    )
    assert all(row[2] == "allowed" for row in rows), (
        f"all benchmark events should be allowed: {rows}"
    )

    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    try:
        leaked = conn.execute(
            """
            SELECT COUNT(*)
            FROM net_events
            WHERE coalesce(request_headers, '') LIKE '%capsem_test_%'
               OR coalesce(response_headers, '') LIKE '%capsem_test_%'
               OR coalesce(request_body_preview, '') LIKE '%capsem_test_%'
               OR coalesce(response_body_preview, '') LIKE '%capsem_test_%'
            """
        ).fetchone()[0]
    finally:
        conn.close()
    assert leaked == 0, "raw synthetic credential marker leaked into session.db"


def test_mitm_local_benchmark_artifact():
    if os.environ.get("CAPSEM_RUN_MITM_LOCAL_BENCH") != "1":
        pytest.skip("set CAPSEM_RUN_MITM_LOCAL_BENCH=1 to run the VM benchmark")

    upstream_proc = None
    base_url = os.environ.get("CAPSEM_BENCH_MITM_LOCAL_BASE_URL")
    if not base_url:
        upstream_proc, ready = _start_debug_upstream()
        base_url = ready["base_url"]
    parsed_base = urlsplit(base_url)
    if parsed_base.hostname != "127.0.0.1" or (parsed_base.port or 80) != 11434:
        pytest.skip(
            "mitm-local benchmark release proof requires "
            "CAPSEM_BENCH_MITM_LOCAL_BASE_URL=http://127.0.0.1:11434 "
            "so guest traffic traverses iptables-nft redirection"
        )

    total_requests = int(os.environ.get("CAPSEM_BENCH_MITM_LOCAL_N", "10"))
    concurrency = int(os.environ.get("CAPSEM_BENCH_MITM_LOCAL_CONCURRENCY", "1"))

    svc = ServiceInstance()
    _write_local_benchmark_policy(svc.tmp_dir, base_url)
    svc.start()
    client = svc.client()
    name = f"mitm-local-{uuid.uuid4().hex[:8]}"

    try:
        client.post("/vms/create", {
            "name": name,
            "ram_mb": DEFAULT_RAM_MB,
            "cpus": DEFAULT_CPUS,
        })
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT), (
            f"{name} not ready"
        )

        command = shlex.join(
            [
                "env",
                f"CAPSEM_BENCH_MITM_LOCAL_BASE_URL={base_url}",
                "capsem-bench",
                "mitm-local",
                base_url,
                str(total_requests),
                str(concurrency),
            ]
        )
        resp = client.post(
            f"/vms/{name}/exec",
            {"command": command, "timeout_secs": 300},
            timeout=310,
        )
        assert resp and resp.get("exit_code") == 0, (
            f"capsem-bench mitm-local failed: "
            f"exit={resp.get('exit_code') if resp else None}\n"
            f"stdout: {(resp or {}).get('stdout', '')[:1000]}\n"
            f"stderr: {(resp or {}).get('stderr', '')[:1000]}"
        )

        resp = client.post(
            f"/vms/{name}/exec",
            {"command": "cat /tmp/capsem-benchmark.json", "timeout_secs": 15},
            timeout=20,
        )
        assert resp and resp.get("exit_code") == 0, (
            "capsem-bench mitm-local did not write /tmp/capsem-benchmark.json"
        )
        data = json.loads(resp.get("stdout", "").strip())
        _assert_mitm_local_succeeded(data)
        assert "capsem_test_api_key" not in json.dumps(data)
        _assert_session_db_contains_mitm_events(svc.tmp_dir, name, total_requests)

        data["host_recorded_at"] = time.time()
        data["arch"] = os.uname().machine
        data["debug_upstream_base_url"] = base_url
        _archive(data)
    finally:
        try:
            client.delete(f"/vms/{name}/delete")
        except Exception:
            pass
        svc.stop()
        _stop_process(upstream_proc)
