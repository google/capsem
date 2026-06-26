"""Archive an in-VM local mock-server protocol benchmark artifact.

The release gate runs this every time. When no explicit
CAPSEM_MOCK_SERVER_BASE_URL is supplied, the test starts the shared mock server
on host localhost and passes that URL to the guest.
"""

import json
import os
import re
import shlex
import sqlite3
import time
import uuid
from pathlib import Path
from urllib.parse import urlsplit

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.mock_server import start_mock_server, stop_process
from helpers.service import ServiceInstance, vm_session_db_path, wait_exec_ready

pytestmark = [pytest.mark.serial, pytest.mark.benchmark]

PROJECT_ROOT = Path(__file__).parent.parent.parent
RELEASE_SCENARIOS = ("model_json_response", "credential_response")
SCENARIO_PATHS = {
    "tiny_http": "/tiny",
    "http_1mb": "/bytes/1mb",
    "gzip_1mb": "/gzip/1mb",
    "sse_model": "/sse/model",
    "model_json_response": "/model/response",
    "denied_target": "/deny-target",
    "credential_response": "/credential/response",
}


def _project_version():
    cargo = PROJECT_ROOT / "Cargo.toml"
    match = re.search(r'^version\s*=\s*"([^"]+)"', cargo.read_text(), re.MULTILINE)
    return match.group(1) if match else "unknown"


def _archive(data):
    version = _project_version()
    arch = "arm64" if os.uname().machine == "arm64" else "x86_64"
    out_dir = PROJECT_ROOT / "benchmarks" / "mock-server-protocol"
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / f"data_{version}_{arch}.json"
    with open(out_path, "w") as handle:
        json.dump(data, handle, indent=2)
    print(f"mock-server-protocol benchmark archived to {out_path}")
    return out_path


def _assert_mock_server_protocol_succeeded(data):
    assert "mock_server_protocol" in data
    result = data["mock_server_protocol"]
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


def _assert_session_db_contains_protocol_events(
    capsem_home, client, vm_name, total_requests, selected_scenarios
):
    db_path = vm_session_db_path(capsem_home, client, vm_name, must_exist=False)
    expected_paths = {SCENARIO_PATHS[name] for name in selected_scenarios}
    expected_paths.update({"/ws/echo", "/ws/close"})
    expected_count = total_requests * len(selected_scenarios) + 2

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
        f"expected at least {expected_count} local mock-server protocol net_events, got {len(rows)}: {rows}"
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


def test_mock_server_protocol_benchmark_artifact():
    upstream_proc = None
    base_url = os.environ.get("CAPSEM_MOCK_SERVER_BASE_URL")
    if not base_url:
        upstream_proc, ready = start_mock_server(capture_requests=False)
        base_url = ready["base_url"]
        assert ready["request_log"] is None, (
            "release protocol benchmark must run capsem-mock-server in perf mode; "
            "request capture serializes every request and poisons tiny_http numbers"
        )
    parsed_base = urlsplit(base_url)
    assert parsed_base.hostname == "127.0.0.1"
    assert (parsed_base.port or 80) == 3713

    total_requests = int(os.environ.get("CAPSEM_BENCH_TOTAL_REQUESTS", "50000"))
    concurrency = int(os.environ.get("CAPSEM_BENCH_CONCURRENCY", "64"))
    selected_scenarios = tuple(
        name.strip()
        for name in os.environ.get(
            "CAPSEM_BENCH_SCENARIOS",
            ",".join(RELEASE_SCENARIOS),
        ).split(",")
        if name.strip()
    )
    assert selected_scenarios, "release benchmark must select at least one scenario"

    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    name = f"mock-server-protocol-{uuid.uuid4().hex[:8]}"

    try:
        client.post("/vms/create", {
            "name": name,
            "profile_id": "code",
            "ram_mb": DEFAULT_RAM_MB,
            "cpus": DEFAULT_CPUS,
        })
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT), (
            f"{name} not ready"
        )

        command = shlex.join(
            [
                "env",
                f"CAPSEM_MOCK_SERVER_BASE_URL={base_url}",
                f"CAPSEM_BENCH_TOTAL_REQUESTS={total_requests}",
                f"CAPSEM_BENCH_CONCURRENCY={concurrency}",
                f"CAPSEM_BENCH_SCENARIOS={','.join(selected_scenarios)}",
                "capsem-bench-rs",
                "protocol",
            ]
        )
        resp = client.post(
            f"/vms/{name}/exec",
            {"command": command, "timeout_secs": 300},
            timeout=310,
        )
        assert resp and resp.get("exit_code") == 0, (
            f"capsem-bench-rs protocol failed to run local protocol scenarios: "
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
            "capsem-bench-rs protocol did not write /tmp/capsem-benchmark.json"
        )
        data = json.loads(resp.get("stdout", "").strip())
        _assert_mock_server_protocol_succeeded(data)
        assert tuple(data["mock_server_protocol"]["selected_scenarios"]) == selected_scenarios
        assert "capsem_test_api_key" not in json.dumps(data)
        _assert_session_db_contains_protocol_events(
            svc.tmp_dir, client, name, total_requests, selected_scenarios
        )

        data["host_recorded_at"] = time.time()
        data["arch"] = os.uname().machine
        data["mock_server_base_url"] = base_url
        _archive(data)
    finally:
        try:
            client.delete(f"/vms/{name}/delete")
        except Exception:
            pass
        svc.stop()
        stop_process(upstream_proc)
