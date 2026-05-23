"""Host-side Security Engine enforcement benchmarks.

This profiles real VM-originated enforcement paths:

- process exec: service API -> capsem-process IPC -> SecurityEvent projection
  -> CEL rule evaluation -> process decision -> session DB/log projection ->
  runtime counters.
- HTTP request: guest curl -> network transport/MITM -> SecurityEvent
  projection -> CEL rule evaluation -> block response -> session DB/log
  projection -> runtime counters.
- DNS request: guest resolver -> capsem DNS proxy -> SecurityEvent projection
  -> CEL rule evaluation -> NXDOMAIN response -> session DB/log projection ->
  runtime counters.
- MCP request: guest capsem-mcp-server -> framed vsock MCP endpoint ->
  SecurityEvent projection -> CEL rule evaluation -> JSON-RPC denial ->
  session DB/log projection -> runtime counters.

The Criterion benchmark in capsem-security-engine measures raw evaluator cost.
This file records the product path cost from a VM-originated security event.
"""

import base64
from contextlib import closing
import json
import os
import re
import shlex
import sqlite3
import statistics
import subprocess
import time
import uuid
from pathlib import Path

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import ServiceInstance, wait_exec_ready

pytestmark = [pytest.mark.serial, pytest.mark.benchmark]

PROJECT_ROOT = Path(__file__).parent.parent.parent

RUNS = int(os.environ.get("CAPSEM_SECURITY_ENGINE_BENCH_RUNS", "8"))
HTTP_WARMUP_RUNS = int(os.environ.get("CAPSEM_SECURITY_ENGINE_HTTP_WARMUP_RUNS", "1"))
BLOCKED_EXEC_GATE_MS = 750
BLOCKED_HTTP_GATE_MS = 1000
BLOCKED_DNS_GATE_MS = 1000
BLOCKED_MCP_GATE_MS = 1000


def _project_version():
    cargo = PROJECT_ROOT / "Cargo.toml"
    m = re.search(r'^version\s*=\s*"([^"]+)"', cargo.read_text(), re.MULTILINE)
    return m.group(1) if m else "unknown"


def _source_commit():
    try:
        result = subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"],
            cwd=PROJECT_ROOT,
            capture_output=True,
            text=True,
            timeout=5,
            check=True,
        )
        return result.stdout.strip()
    except Exception:
        return "unknown"


def _save_security_engine_benchmark(data, suffix):
    version = _project_version()
    arch = "arm64" if os.uname().machine == "arm64" else "x86_64"
    out_dir = PROJECT_ROOT / "benchmarks" / "security-engine"
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / f"data_{version}_{arch}_{suffix}.json"
    out_path.write_text(json.dumps(data, indent=2) + "\n")
    print(f"Security Engine benchmark saved to {out_path}")


def _percentile(sorted_values, percentile):
    if not sorted_values:
        raise ValueError("percentile requires at least one value")
    index = round((len(sorted_values) - 1) * percentile)
    return sorted_values[index]


def _series_summary(values):
    sorted_values = sorted(values)
    return {
        "min": round(min(values), 3),
        "mean": round(statistics.mean(values), 3),
        "median": round(statistics.median(values), 3),
        "p95": round(_percentile(sorted_values, 0.95), 3),
        "p99": round(_percentile(sorted_values, 0.99), 3),
        "max": round(max(values), 3),
        "values": [round(value, 3) for value in values],
    }


def _session_db_path(service, vm):
    candidates = [
        service.tmp_dir / "sessions" / vm / "session.db",
        service.tmp_dir / "persistent" / vm / "session.db",
        *sorted((service.tmp_dir / "sessions").glob(f"{vm}*/session.db")),
    ]
    return next((path for path in candidates if path.exists()), None)


def _guest_python(script):
    encoded = base64.b64encode(script.encode()).decode()
    command = f"import base64; exec(base64.b64decode({encoded!r}).decode())"
    return f"python3 -c {shlex.quote(command)}"


def _wait_for_security_event_count(
    service,
    vm,
    rule_id,
    expected,
    *,
    event_type,
    timeout=10.0,
):
    deadline = time.time() + timeout
    last_error = "security event count was never queried"
    while time.time() < deadline:
        db_path = _session_db_path(service, vm)
        if db_path is None:
            last_error = f"session.db for {vm} does not exist yet"
            time.sleep(0.25)
            continue

        try:
            with closing(sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)) as conn:
                row = conn.execute(
                    """
                    SELECT COUNT(*),
                           COUNT(DISTINCT se.event_id),
                           SUM(CASE WHEN se.final_action = 'block' THEN 1 ELSE 0 END),
                           MIN(se.vm_id),
                           MIN(se.profile_id),
                           MIN(se.user_id),
                           MIN(se.process_operation),
                           MIN(se.process_command_class),
                           MIN(step.rule_id),
                           MIN(step.message)
                     FROM security_events se
                      JOIN security_event_steps step
                        ON step.event_id = se.event_id
                     WHERE se.event_type = ?
                       AND step.rule_id = ?
                    """,
                    (event_type, rule_id),
                ).fetchone()
                all_rows = conn.execute(
                    """
                    SELECT se.event_id,
                           se.final_action,
                           COALESCE(step.rule_id, ''),
                           COALESCE(step.message, '')
                      FROM security_events se
                      LEFT JOIN security_event_steps step
                        ON step.event_id = se.event_id
                     WHERE se.event_type = ?
                     ORDER BY se.rowid
                    """,
                    (event_type,),
                ).fetchall()
        except sqlite3.Error as error:
            last_error = str(error)
            time.sleep(0.25)
            continue

        count = int(row[0] or 0)
        if count >= expected:
            return {
                "row_count": count,
                "distinct_event_ids": int(row[1] or 0),
                "blocked_count": int(row[2] or 0),
                "vm_id": row[3],
                "profile_id": row[4],
                "user_id": row[5],
                "process_operation": row[6],
                "process_command_class": row[7],
                "rule_id": row[8],
                "reason": row[9],
            }
        last_error = f"only {count}/{expected} {event_type} security events for {rule_id}"
        if all_rows:
            last_error += f"; rows={all_rows}"
        time.sleep(0.25)

    raise AssertionError(last_error)


def _wait_for_dns_event_count(service, vm, qname, expected, timeout=10.0):
    deadline = time.time() + timeout
    last_error = "dns event count was never queried"
    while time.time() < deadline:
        db_path = _session_db_path(service, vm)
        if db_path is None:
            last_error = f"session.db for {vm} does not exist yet"
            time.sleep(0.25)
            continue

        try:
            with closing(sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)) as conn:
                row = conn.execute(
                    """
                    SELECT COUNT(*),
                           SUM(CASE WHEN decision = 'denied' THEN 1 ELSE 0 END),
                           MIN(qname),
                           MIN(policy_mode),
                           MIN(policy_action),
                           MIN(policy_rule),
                           MIN(policy_reason)
                      FROM dns_events
                     WHERE qname = ?
                    """,
                    (qname,),
                ).fetchone()
        except sqlite3.Error as error:
            last_error = str(error)
            time.sleep(0.25)
            continue

        count = int(row[0] or 0)
        if count >= expected:
            return {
                "row_count": count,
                "denied_count": int(row[1] or 0),
                "qname": row[2],
                "policy_mode": row[3],
                "policy_action": row[4],
                "policy_rule": row[5],
                "policy_reason": row[6],
            }
        last_error = f"only {count}/{expected} dns_events rows for {qname}"
        time.sleep(0.25)

    raise AssertionError(last_error)


def _wait_for_mcp_call_count(service, vm, server_name, tool_name, expected, timeout=10.0):
    deadline = time.time() + timeout
    last_error = "mcp call count was never queried"
    while time.time() < deadline:
        db_path = _session_db_path(service, vm)
        if db_path is None:
            last_error = f"session.db for {vm} does not exist yet"
            time.sleep(0.25)
            continue

        try:
            with closing(sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)) as conn:
                row = conn.execute(
                    """
                    SELECT COUNT(*),
                           SUM(CASE WHEN decision = 'denied' THEN 1 ELSE 0 END),
                           MIN(server_name),
                           MIN(tool_name),
                           MIN(policy_mode),
                           MIN(policy_action),
                           MIN(policy_rule),
                           MIN(policy_reason)
                      FROM mcp_calls
                     WHERE server_name = ?
                       AND tool_name = ?
                    """,
                    (server_name, tool_name),
                ).fetchone()
        except sqlite3.Error as error:
            last_error = str(error)
            time.sleep(0.25)
            continue

        count = int(row[0] or 0)
        if count >= expected:
            return {
                "row_count": count,
                "denied_count": int(row[1] or 0),
                "server_name": row[2],
                "tool_name": row[3],
                "policy_mode": row[4],
                "policy_action": row[5],
                "policy_rule": row[6],
                "policy_reason": row[7],
            }
        last_error = f"only {count}/{expected} mcp_calls rows for {tool_name}"
        time.sleep(0.25)

    raise AssertionError(last_error)


def _runtime_rule_stats(client, rule_id):
    stats = client.get("/enforcement/stats")
    rules = stats.get("rules", [])
    matches = [rule for rule in rules if rule.get("id") == rule_id]
    assert matches, stats
    return matches[0]


def test_process_enforcement_benchmark_records_vm_originated_path():
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    vm = f"secbench-{uuid.uuid4().hex[:8]}"
    rule_id = f"runtime.block-shell-bench.{uuid.uuid4().hex[:8]}"
    reason = "shell exec blocked by security benchmark"

    durations_ms = []
    try:
        client.post(
            "/provision",
            {"name": vm, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS},
            timeout=180,
        )
        assert wait_exec_ready(client, vm, timeout=EXEC_READY_TIMEOUT), (
            f"{vm} never became exec-ready"
        )

        install = client.post(
            "/enforcement",
            {
                "id": rule_id,
                "pack_id": "runtime-benchmark",
                "condition": (
                    "process.activity.operation == 'exec' "
                    "&& process.activity.command_class == 'shell'"
                ),
                "decision": "block",
                "reason": reason,
                "enabled": True,
            },
            timeout=60,
        )
        assert install["rule"]["id"] == rule_id
        assert install["rule"]["compiled"] is True

        for index in range(RUNS):
            started = time.perf_counter()
            resp = client.post(
                f"/exec/{vm}",
                {
                    "command": f"bash -lc 'echo should-not-run-{index}'",
                    "timeout_secs": 5,
                },
                timeout=15,
            )
            durations_ms.append((time.perf_counter() - started) * 1000)
            combined = (resp or {}).get("stdout", "") + (resp or {}).get("stderr", "")
            assert resp and resp.get("exit_code") != 0, resp
            assert "process exec blocked" in combined
            assert rule_id in combined

        db_summary = _wait_for_security_event_count(
            svc,
            vm,
            rule_id,
            RUNS,
            event_type="process.exec",
        )
        assert db_summary["row_count"] >= RUNS
        assert db_summary["distinct_event_ids"] >= RUNS
        assert db_summary["blocked_count"] >= RUNS
        assert db_summary["vm_id"] == vm
        assert db_summary["profile_id"]
        assert db_summary["user_id"]
        assert db_summary["process_operation"] == "exec"
        assert db_summary["process_command_class"] == "shell"
        assert db_summary["rule_id"] == rule_id
        assert db_summary["reason"] == reason

        rule_stats = _runtime_rule_stats(client, rule_id)
        assert rule_stats["match_count"] >= RUNS
        last_matched_event = rule_stats["last_matched_event"]
        assert isinstance(last_matched_event, str)
        assert last_matched_event.startswith("process-")

        logs = client.get(f"/logs/{vm}")
        security_logs = logs.get("security_logs") or ""
        process_logs = logs.get("process_logs") or ""
        combined_logs = security_logs + "\n" + process_logs
        assert "process_exec_security_decision" in combined_logs
        assert f'"rule_id":"{rule_id}"' in combined_logs
        assert f'"vm_id":"{vm}"' in combined_logs
        assert '"event_type":"process.exec"' in combined_logs
        assert '"final_action":"block"' in combined_logs

        sorted_durations = sorted(durations_ms)
        summary = {
            "schema": "capsem.security-engine-benchmark.v1",
            "kind": "vm_originated_process_enforcement",
            "version": _project_version(),
            "source_commit": _source_commit(),
            "timestamp": time.time(),
            "arch": os.uname().machine,
            "host": {
                "sysname": os.uname().sysname,
                "release": os.uname().release,
                "machine": os.uname().machine,
            },
            "command": (
                "uv run pytest "
                "tests/capsem-serial/test_security_engine_benchmark.py -xvs"
            ),
            "workload": {
                "event_family": "process",
                "event_type": "process.exec",
                "source": "vm_originated",
                "path": "service_api_to_capsem_process_to_security_engine",
            },
            "runs": RUNS,
            "gate_ms": BLOCKED_EXEC_GATE_MS,
            "rule": {
                "id": rule_id,
                "pack_id": "runtime-benchmark",
                "condition": install["rule"]["condition"],
                "decision": "block",
            },
            "operations": {
                "blocked_process_exec_ms": {
                    "min": round(min(durations_ms), 3),
                    "mean": round(statistics.mean(durations_ms), 3),
                    "median": round(statistics.median(durations_ms), 3),
                    "p95": round(_percentile(sorted_durations, 0.95), 3),
                    "p99": round(_percentile(sorted_durations, 0.99), 3),
                    "max": round(max(durations_ms), 3),
                    "values": [round(value, 3) for value in durations_ms],
                }
            },
            "assertions": {
                "session_db_security_events": db_summary,
                "runtime_match_count": rule_stats["match_count"],
                "runtime_last_event_id": last_matched_event,
                "logs_exposed_security_decision": True,
            },
        }
        _save_security_engine_benchmark(summary, "process_enforcement")

        mean_ms = summary["operations"]["blocked_process_exec_ms"]["mean"]
        assert mean_ms < BLOCKED_EXEC_GATE_MS, (
            f"blocked process exec mean {mean_ms:.0f}ms exceeds "
            f"{BLOCKED_EXEC_GATE_MS}ms gate"
        )
    finally:
        try:
            client.delete(f"/enforcement/{rule_id}", timeout=60)
        except Exception:
            pass
        try:
            client.delete(f"/delete/{vm}", timeout=120)
        except Exception:
            pass
        svc.stop()


def test_http_request_enforcement_benchmark_records_vm_originated_path():
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    vm = f"sechttp-{uuid.uuid4().hex[:8]}"
    rule_id = f"runtime.block-http-bench.{uuid.uuid4().hex[:8]}"
    reason = "HTTP request blocked by security benchmark"
    path = f"/security-engine-bench-block-{uuid.uuid4().hex[:8]}"

    try:
        client.post(
            "/provision",
            {"name": vm, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS},
            timeout=180,
        )
        assert wait_exec_ready(client, vm, timeout=EXEC_READY_TIMEOUT), (
            f"{vm} never became exec-ready"
        )

        install = client.post(
            "/enforcement",
            {
                "id": rule_id,
                "pack_id": "runtime-benchmark",
                "condition": (
                    "http.request.host == 'example.com' "
                    f"&& http.request.path == '{path}'"
                ),
                "decision": "block",
                "reason": reason,
                "enabled": True,
            },
            timeout=60,
        )
        assert install["rule"]["id"] == rule_id
        assert install["rule"]["compiled"] is True

        script = f"""
import json
import socket
import ssl
import subprocess
import time

runs = {RUNS}
warmup_runs = {HTTP_WARMUP_RUNS}
url = "https://example.com{path}"
durations_ms = []
starttransfer_ms = []
curl_phase_ms = {{
    "namelookup": [],
    "connect": [],
    "appconnect": [],
    "pretransfer": [],
    "starttransfer": [],
    "total": [],
}}
curl_phase_delta_ms = {{
    "dns": [],
    "tcp_connect": [],
    "tls_appconnect": [],
    "pretransfer_after_tls": [],
    "server_first_byte_after_pretransfer": [],
    "response_tail_after_first_byte": [],
}}
keepalive_starttransfer_ms = []
keepalive_total_ms = []
keepalive_results = []
keepalive_connection = {{}}
results = []

def run_once():
    started = time.perf_counter()
    proc = subprocess.run(
        [
            "curl",
            "-k",
            "-sS",
            "--max-time",
            "15",
            "-w",
            "\\nHTTP_STATUS:%{{http_code}}"
            "\\nTIME_NAMELOOKUP:%{{time_namelookup}}"
            "\\nTIME_CONNECT:%{{time_connect}}"
            "\\nTIME_APPCONNECT:%{{time_appconnect}}"
            "\\nTIME_PRETRANSFER:%{{time_pretransfer}}"
            "\\nTIME_STARTTRANSFER:%{{time_starttransfer}}"
            "\\nTIME_TOTAL:%{{time_total}}",
            url,
        ],
        capture_output=True,
        text=True,
        timeout=20,
    )
    wall_ms = (time.perf_counter() - started) * 1000
    status = None
    phases = {{}}
    for line in proc.stdout.splitlines():
        if line.startswith("HTTP_STATUS:"):
            status = line.split(":", 1)[1]
        if line.startswith("TIME_"):
            key, value = line.split(":", 1)
            phases[key[5:].lower()] = float(value) * 1000
    deltas = {{
        "dns": phases["namelookup"],
        "tcp_connect": phases["connect"] - phases["namelookup"],
        "tls_appconnect": phases["appconnect"] - phases["connect"],
        "pretransfer_after_tls": phases["pretransfer"] - phases["appconnect"],
        "server_first_byte_after_pretransfer": (
            phases["starttransfer"] - phases["pretransfer"]
        ),
        "response_tail_after_first_byte": phases["total"] - phases["starttransfer"],
    }}
    return wall_ms, phases, deltas, {{
        "returncode": proc.returncode,
        "stdout": proc.stdout,
        "stderr": proc.stderr,
        "http_status": status,
        "curl_phase_ms": phases,
        "curl_phase_delta_ms": deltas,
    }}

def run_keepalive():
    connect_started = time.perf_counter()
    raw = socket.create_connection(("example.com", 443), timeout=15)
    connect_ms = (time.perf_counter() - connect_started) * 1000
    context = ssl._create_unverified_context()
    tls_started = time.perf_counter()
    conn = context.wrap_socket(raw, server_hostname="example.com")
    tls_ms = (time.perf_counter() - tls_started) * 1000
    transfers = []
    try:
        for index in range(runs):
            request = (
                f"GET {path} HTTP/1.1\\r\\n"
                "Host: example.com\\r\\n"
                "User-Agent: capsem-security-bench\\r\\n"
                "Accept: */*\\r\\n"
                f"Connection: {{'close' if index == runs - 1 else 'keep-alive'}}\\r\\n"
                "\\r\\n"
            ).encode()
            started = time.perf_counter()
            conn.sendall(request)
            buffer = b""
            first_byte_ms = None
            while b"\\r\\n\\r\\n" not in buffer:
                chunk = conn.recv(4096)
                if not chunk:
                    raise RuntimeError("connection closed before response headers")
                if first_byte_ms is None:
                    first_byte_ms = (time.perf_counter() - started) * 1000
                buffer += chunk
            header_bytes, body = buffer.split(b"\\r\\n\\r\\n", 1)
            headers_text = header_bytes.decode("iso-8859-1")
            status_line = headers_text.split("\\r\\n", 1)[0]
            content_length = 0
            for line in headers_text.split("\\r\\n")[1:]:
                name, _, value = line.partition(":")
                if name.lower() == "content-length":
                    content_length = int(value.strip())
            while len(body) < content_length:
                chunk = conn.recv(4096)
                if not chunk:
                    raise RuntimeError("connection closed before response body")
                body += chunk
            total_ms = (time.perf_counter() - started) * 1000
            transfers.append({{
                "http_status": status_line.split()[1],
                "starttransfer": first_byte_ms,
                "total": total_ms,
                "body": body[:content_length].decode("utf-8", "replace"),
            }})
    finally:
        conn.close()
    return {{
        "returncode": 0,
        "connect_ms": connect_ms,
        "tls_handshake_ms": tls_ms,
        "transfers": transfers,
    }}

for _ in range(warmup_runs):
    run_once()

for index in range(runs):
    wall_ms, phases, deltas, result = run_once()
    durations_ms.append(wall_ms)
    starttransfer_ms.append(phases["starttransfer"])
    for name, value in phases.items():
        curl_phase_ms[name].append(value)
    for name, value in deltas.items():
        curl_phase_delta_ms[name].append(value)
    results.append(result)

keepalive_payload = run_keepalive()
keepalive_connection = {{
    "connect_ms": keepalive_payload["connect_ms"],
    "tls_handshake_ms": keepalive_payload["tls_handshake_ms"],
}}
for transfer in keepalive_payload["transfers"]:
    keepalive_results.append(transfer)
    keepalive_starttransfer_ms.append(transfer["starttransfer"])
    keepalive_total_ms.append(transfer["total"])

print(json.dumps({{
    "warmup_runs": warmup_runs,
    "durations_ms": durations_ms,
    "starttransfer_ms": starttransfer_ms,
    "curl_phase_ms": curl_phase_ms,
    "curl_phase_delta_ms": curl_phase_delta_ms,
    "keepalive_starttransfer_ms": keepalive_starttransfer_ms,
    "keepalive_total_ms": keepalive_total_ms,
    "keepalive_connection": keepalive_connection,
    "keepalive_result": keepalive_payload,
    "keepalive_results": keepalive_results,
    "results": results,
}}))
"""
        response = client.post(
            f"/exec/{vm}",
            {"command": _guest_python(script), "timeout_secs": 180},
            timeout=195,
        )
        assert response is not None and response.get("exit_code") == 0, response
        payload = json.loads(response["stdout"].strip().splitlines()[-1])
        durations_ms = payload["durations_ms"]
        starttransfer_ms = payload["starttransfer_ms"]
        curl_phase_ms = payload["curl_phase_ms"]
        curl_phase_delta_ms = payload["curl_phase_delta_ms"]
        keepalive_starttransfer_ms = payload["keepalive_starttransfer_ms"]
        keepalive_total_ms = payload["keepalive_total_ms"]
        keepalive_connection = payload["keepalive_connection"]
        keepalive_result = payload["keepalive_result"]
        assert len(durations_ms) == RUNS
        assert len(starttransfer_ms) == RUNS
        assert all(len(values) == RUNS for values in curl_phase_ms.values())
        assert all(len(values) == RUNS for values in curl_phase_delta_ms.values())
        assert keepalive_result["returncode"] == 0, keepalive_result
        assert len(keepalive_starttransfer_ms) == RUNS, keepalive_result
        assert len(keepalive_total_ms) == RUNS, keepalive_result
        assert keepalive_connection["connect_ms"] >= 0, keepalive_result
        assert keepalive_connection["tls_handshake_ms"] >= 0, keepalive_result
        assert all(
            result["http_status"] == "403"
            for result in keepalive_result["transfers"]
        ), keepalive_result
        assert all(
            reason in result["body"] for result in keepalive_result["transfers"]
        ), keepalive_result
        for result in payload["results"]:
            assert result["returncode"] == 0, result
            assert result["http_status"] == "403", result
            assert result["curl_phase_ms"]["starttransfer"] is not None, result
            assert reason in result["stdout"], result

        expected_security_events = HTTP_WARMUP_RUNS + RUNS + RUNS
        db_summary = _wait_for_security_event_count(
            svc,
            vm,
            rule_id,
            expected_security_events,
            event_type="http.request",
            timeout=20.0,
        )
        assert db_summary["row_count"] >= expected_security_events
        assert db_summary["distinct_event_ids"] >= expected_security_events
        assert db_summary["blocked_count"] >= expected_security_events
        assert db_summary["vm_id"] == vm
        assert db_summary["profile_id"]
        assert db_summary["user_id"]
        assert db_summary["rule_id"] == rule_id
        assert db_summary["reason"] == reason

        rule_stats = _runtime_rule_stats(client, rule_id)
        assert rule_stats["match_count"] >= RUNS
        last_matched_event = rule_stats["last_matched_event"]
        assert isinstance(last_matched_event, str)
        assert last_matched_event.startswith("net-http-")

        logs = client.get(f"/logs/{vm}")
        security_logs = logs.get("security_logs") or ""
        assert f'"rule_id":"{rule_id}"' in security_logs
        assert f'"vm_id":"{vm}"' in security_logs
        assert '"event_type":"http.request"' in security_logs
        assert '"final_action":"block"' in security_logs

        summary = {
            "schema": "capsem.security-engine-benchmark.v1",
            "kind": "vm_originated_http_request_enforcement",
            "version": _project_version(),
            "source_commit": _source_commit(),
            "timestamp": time.time(),
            "arch": os.uname().machine,
            "host": {
                "sysname": os.uname().sysname,
                "release": os.uname().release,
                "machine": os.uname().machine,
            },
            "command": (
                "uv run pytest tests/capsem-serial/"
                "test_security_engine_benchmark.py::"
                "test_http_request_enforcement_benchmark_records_vm_originated_path -xvs"
            ),
            "workload": {
                "event_family": "network",
                "event_type": "http.request",
                "source": "vm_originated",
                "path": "guest_curl_to_mitm_to_security_engine",
            },
            "runs": RUNS,
            "warmup_runs": payload["warmup_runs"],
            "keepalive_runs": RUNS,
            "gate_ms": BLOCKED_HTTP_GATE_MS,
            "rule": {
                "id": rule_id,
                "pack_id": "runtime-benchmark",
                "condition": install["rule"]["condition"],
                "decision": "block",
            },
            "operations": {
                "blocked_http_request_wall_ms": _series_summary(durations_ms),
                "blocked_http_request_starttransfer_ms": _series_summary(
                    starttransfer_ms
                ),
                "curl_phase_ms": {
                    name: _series_summary(values)
                    for name, values in sorted(curl_phase_ms.items())
                },
                "curl_phase_delta_ms": {
                    name: _series_summary(values)
                    for name, values in sorted(curl_phase_delta_ms.items())
                },
                "keepalive_http_request_starttransfer_ms": _series_summary(
                    keepalive_starttransfer_ms
                ),
                "keepalive_http_request_total_ms": _series_summary(keepalive_total_ms),
                "keepalive_connection_ms": {
                    name: round(value, 3)
                    for name, value in sorted(keepalive_connection.items())
                },
            },
            "assertions": {
                "session_db_security_events": db_summary,
                "runtime_match_count": rule_stats["match_count"],
                "runtime_last_event_id": last_matched_event,
                "logs_exposed_security_decision": True,
            },
        }
        _save_security_engine_benchmark(summary, "http_request_enforcement")

        mean_ms = summary["operations"]["blocked_http_request_starttransfer_ms"]["mean"]
        assert mean_ms < BLOCKED_HTTP_GATE_MS, (
            f"blocked HTTP request time-starttransfer mean {mean_ms:.0f}ms exceeds "
            f"{BLOCKED_HTTP_GATE_MS}ms gate"
        )
    finally:
        try:
            client.delete(f"/enforcement/{rule_id}", timeout=60)
        except Exception:
            pass
        try:
            client.delete(f"/delete/{vm}", timeout=120)
        except Exception:
            pass
        svc.stop()


def test_dns_request_enforcement_benchmark_records_vm_originated_path():
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    vm = f"secdns-{uuid.uuid4().hex[:8]}"
    rule_id = f"runtime.block-dns-bench.{uuid.uuid4().hex[:8]}"
    reason = "DNS request blocked by security benchmark"
    qname = f"security-engine-bench-{uuid.uuid4().hex[:8]}.example.com"

    try:
        client.post(
            "/provision",
            {"name": vm, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS},
            timeout=180,
        )
        assert wait_exec_ready(client, vm, timeout=EXEC_READY_TIMEOUT), (
            f"{vm} never became exec-ready"
        )

        install = client.post(
            "/enforcement",
            {
                "id": rule_id,
                "pack_id": "runtime-benchmark",
                "condition": f"dns.request.qname == '{qname}'",
                "decision": "block",
                "reason": reason,
                "enabled": True,
            },
            timeout=60,
        )
        assert install["rule"]["id"] == rule_id
        assert install["rule"]["compiled"] is True

        script = f"""
import json
import socket
import time

runs = {RUNS}
qname = {qname!r}
durations_ms = []
results = []

for index in range(runs):
    started = time.perf_counter()
    try:
        socket.getaddrinfo(qname, 443, type=socket.SOCK_STREAM)
        ok = True
        error = None
    except OSError as exc:
        ok = False
        error = str(exc)
    durations_ms.append((time.perf_counter() - started) * 1000)
    results.append({{"ok": ok, "error": error}})

print(json.dumps({{
    "durations_ms": durations_ms,
    "results": results,
}}))
"""
        response = client.post(
            f"/exec/{vm}",
            {"command": _guest_python(script), "timeout_secs": 120},
            timeout=135,
        )
        assert response is not None and response.get("exit_code") == 0, response
        payload = json.loads(response["stdout"].strip().splitlines()[-1])
        durations_ms = payload["durations_ms"]
        assert len(durations_ms) == RUNS
        for result in payload["results"]:
            assert result["ok"] is False, result
            assert result["error"], result

        db_summary = _wait_for_security_event_count(
            svc,
            vm,
            rule_id,
            RUNS,
            event_type="dns.request",
            timeout=20.0,
        )
        assert db_summary["row_count"] >= RUNS
        assert db_summary["distinct_event_ids"] >= RUNS
        assert db_summary["blocked_count"] >= RUNS
        assert db_summary["vm_id"] == vm
        assert db_summary["profile_id"]
        assert db_summary["user_id"]
        assert db_summary["rule_id"] == rule_id
        assert db_summary["reason"] == reason

        dns_summary = _wait_for_dns_event_count(svc, vm, qname, RUNS, timeout=20.0)
        assert dns_summary["row_count"] >= RUNS
        assert dns_summary["denied_count"] >= RUNS
        assert dns_summary["qname"] == qname
        assert dns_summary["policy_mode"] == "runtime"
        assert dns_summary["policy_action"] == "block"
        assert dns_summary["policy_rule"] == rule_id
        assert dns_summary["policy_reason"] == reason

        rule_stats = _runtime_rule_stats(client, rule_id)
        assert rule_stats["match_count"] >= RUNS
        last_matched_event = rule_stats["last_matched_event"]
        assert isinstance(last_matched_event, str)
        assert last_matched_event.startswith("dns-")

        logs = client.get(f"/logs/{vm}")
        security_logs = logs.get("security_logs") or ""
        dns_logs = logs.get("dns_logs") or ""
        combined_logs = security_logs + "\n" + dns_logs
        assert f'"rule_id":"{rule_id}"' in combined_logs
        assert f'"vm_id":"{vm}"' in combined_logs
        assert '"event_type":"dns.request"' in combined_logs
        assert '"final_action":"block"' in combined_logs
        assert qname in combined_logs

        summary = {
            "schema": "capsem.security-engine-benchmark.v1",
            "kind": "vm_originated_dns_request_enforcement",
            "version": _project_version(),
            "source_commit": _source_commit(),
            "timestamp": time.time(),
            "arch": os.uname().machine,
            "host": {
                "sysname": os.uname().sysname,
                "release": os.uname().release,
                "machine": os.uname().machine,
            },
            "command": (
                "uv run pytest tests/capsem-serial/"
                "test_security_engine_benchmark.py::"
                "test_dns_request_enforcement_benchmark_records_vm_originated_path -xvs"
            ),
            "workload": {
                "event_family": "dns",
                "event_type": "dns.request",
                "source": "vm_originated",
                "path": "guest_resolver_to_dns_proxy_to_security_engine",
            },
            "runs": RUNS,
            "gate_ms": BLOCKED_DNS_GATE_MS,
            "rule": {
                "id": rule_id,
                "pack_id": "runtime-benchmark",
                "condition": install["rule"]["condition"],
                "decision": "block",
            },
            "operations": {
                "blocked_dns_request_ms": _series_summary(durations_ms),
            },
            "assertions": {
                "session_db_security_events": db_summary,
                "session_db_dns_events": dns_summary,
                "runtime_match_count": rule_stats["match_count"],
                "runtime_last_event_id": last_matched_event,
                "logs_exposed_security_decision": True,
            },
        }
        _save_security_engine_benchmark(summary, "dns_request_enforcement")

        mean_ms = summary["operations"]["blocked_dns_request_ms"]["mean"]
        assert mean_ms < BLOCKED_DNS_GATE_MS, (
            f"blocked DNS request mean {mean_ms:.0f}ms exceeds "
            f"{BLOCKED_DNS_GATE_MS}ms gate"
        )
    finally:
        try:
            client.delete(f"/enforcement/{rule_id}", timeout=60)
        except Exception:
            pass
        try:
            client.delete(f"/delete/{vm}", timeout=120)
        except Exception:
            pass
        svc.stop()


def test_mcp_request_enforcement_benchmark_records_vm_originated_path():
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    vm = f"secmcp-{uuid.uuid4().hex[:8]}"
    rule_id = f"runtime.block-mcp-bench.{uuid.uuid4().hex[:8]}"
    reason = "MCP request blocked by security benchmark"
    server_name = "local"
    tool_name = "echo"
    namespaced_tool = f"{server_name}__{tool_name}"

    try:
        client.post(
            "/provision",
            {"name": vm, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS},
            timeout=180,
        )
        assert wait_exec_ready(client, vm, timeout=EXEC_READY_TIMEOUT), (
            f"{vm} never became exec-ready"
        )

        install = client.post(
            "/enforcement",
            {
                "id": rule_id,
                "pack_id": "runtime-benchmark",
                "condition": (
                    "mcp.request.server_id == 'local' "
                    "&& mcp.request.tool_name == 'echo'"
                ),
                "decision": "block",
                "reason": reason,
                "enabled": True,
            },
            timeout=60,
        )
        assert install["rule"]["id"] == rule_id
        assert install["rule"]["compiled"] is True

        script = f"""
import json
import subprocess
import time

runs = {RUNS}
tool_name = {namespaced_tool!r}
proc = subprocess.Popen(
    ["/run/capsem-mcp-server"],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    text=True,
    bufsize=1,
)

def send(message):
    proc.stdin.write(json.dumps(message) + "\\n")
    proc.stdin.flush()
    line = proc.stdout.readline()
    if not line:
        stderr = proc.stderr.read()
        raise RuntimeError(f"missing MCP response; stderr={{stderr!r}}")
    return json.loads(line)

init = send({{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "initialize",
    "params": {{}},
}})
proc.stdin.write(json.dumps({{
    "jsonrpc": "2.0",
    "method": "notifications/initialized",
}}) + "\\n")
proc.stdin.flush()

durations_ms = []
responses = []
for index in range(runs):
    started = time.perf_counter()
    response = send({{
        "jsonrpc": "2.0",
        "id": index + 2,
        "method": "tools/call",
        "params": {{
            "name": tool_name,
            "arguments": {{"text": f"bench-{{index}}"}},
        }},
    }})
    durations_ms.append((time.perf_counter() - started) * 1000)
    responses.append(response)

proc.terminate()
try:
    proc.wait(timeout=2)
except subprocess.TimeoutExpired:
    proc.kill()

print(json.dumps({{
    "init": init,
    "durations_ms": durations_ms,
    "responses": responses,
}}))
"""
        response = client.post(
            f"/exec/{vm}",
            {"command": _guest_python(script), "timeout_secs": 120},
            timeout=135,
        )
        assert response is not None and response.get("exit_code") == 0, response
        payload = json.loads(response["stdout"].strip().splitlines()[-1])
        durations_ms = payload["durations_ms"]
        assert len(durations_ms) == RUNS
        for rpc_response in payload["responses"]:
            assert "error" in rpc_response, rpc_response
            assert "blocked by policy" in rpc_response["error"]["message"], rpc_response

        db_summary = _wait_for_security_event_count(
            svc,
            vm,
            rule_id,
            RUNS,
            event_type="mcp.request",
            timeout=20.0,
        )
        assert db_summary["row_count"] >= RUNS
        assert db_summary["distinct_event_ids"] >= RUNS
        assert db_summary["blocked_count"] >= RUNS
        assert db_summary["vm_id"] == vm
        assert db_summary["profile_id"]
        assert db_summary["user_id"]
        assert db_summary["rule_id"] == rule_id
        assert db_summary["reason"] == reason

        mcp_summary = _wait_for_mcp_call_count(
            svc,
            vm,
            server_name,
            namespaced_tool,
            RUNS,
            timeout=20.0,
        )
        assert mcp_summary["row_count"] >= RUNS
        assert mcp_summary["denied_count"] >= RUNS
        assert mcp_summary["server_name"] == server_name
        assert mcp_summary["tool_name"] == namespaced_tool
        assert mcp_summary["policy_mode"] == "enforce"
        assert mcp_summary["policy_action"] == "block"
        assert mcp_summary["policy_rule"] == rule_id
        assert mcp_summary["policy_reason"] == reason

        rule_stats = _runtime_rule_stats(client, rule_id)
        assert rule_stats["match_count"] >= RUNS
        last_matched_event = rule_stats["last_matched_event"]
        assert isinstance(last_matched_event, str)
        assert last_matched_event.startswith("mcp-")

        logs = client.get(f"/logs/{vm}")
        security_logs = logs.get("security_logs") or ""
        mcp_logs = logs.get("mcp_logs") or ""
        combined_logs = security_logs + "\n" + mcp_logs
        assert f'"rule_id":"{rule_id}"' in combined_logs
        assert f'"vm_id":"{vm}"' in combined_logs
        assert '"event_type":"mcp.request"' in combined_logs
        assert '"final_action":"block"' in combined_logs
        assert f'"mcp_server_id":"{server_name}"' in combined_logs
        assert f'"mcp_tool_name":"{namespaced_tool}"' in combined_logs

        summary = {
            "schema": "capsem.security-engine-benchmark.v1",
            "kind": "vm_originated_mcp_request_enforcement",
            "version": _project_version(),
            "source_commit": _source_commit(),
            "timestamp": time.time(),
            "arch": os.uname().machine,
            "host": {
                "sysname": os.uname().sysname,
                "release": os.uname().release,
                "machine": os.uname().machine,
            },
            "command": (
                "uv run pytest tests/capsem-serial/"
                "test_security_engine_benchmark.py::"
                "test_mcp_request_enforcement_benchmark_records_vm_originated_path -xvs"
            ),
            "workload": {
                "event_family": "mcp",
                "event_type": "mcp.request",
                "source": "vm_originated",
                "path": "guest_mcp_server_to_framed_vsock_to_security_engine",
            },
            "runs": RUNS,
            "gate_ms": BLOCKED_MCP_GATE_MS,
            "rule": {
                "id": rule_id,
                "pack_id": "runtime-benchmark",
                "condition": install["rule"]["condition"],
                "decision": "block",
            },
            "operations": {
                "blocked_mcp_request_ms": _series_summary(durations_ms),
            },
            "assertions": {
                "session_db_security_events": db_summary,
                "session_db_mcp_calls": mcp_summary,
                "runtime_match_count": rule_stats["match_count"],
                "runtime_last_event_id": last_matched_event,
                "logs_exposed_security_decision": True,
            },
        }
        _save_security_engine_benchmark(summary, "mcp_request_enforcement")

        mean_ms = summary["operations"]["blocked_mcp_request_ms"]["mean"]
        assert mean_ms < BLOCKED_MCP_GATE_MS, (
            f"blocked MCP request mean {mean_ms:.0f}ms exceeds "
            f"{BLOCKED_MCP_GATE_MS}ms gate"
        )
    finally:
        try:
            client.delete(f"/enforcement/{rule_id}", timeout=60)
        except Exception:
            pass
        try:
            client.delete(f"/delete/{vm}", timeout=120)
        except Exception:
            pass
        svc.stop()
