"""Host-side Security Engine enforcement benchmarks.

This profiles the real blocked-exec path:

service API -> capsem-process IPC -> SecurityEvent projection -> CEL rule
evaluation -> process decision -> session DB/log projection -> runtime counters.

The Criterion benchmark in capsem-security-engine measures raw evaluator cost.
This file records the product path cost from a VM-originated security event.
"""

from contextlib import closing
import json
import os
import re
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
BLOCKED_EXEC_GATE_MS = 750


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


def _save_security_engine_benchmark(data):
    version = _project_version()
    arch = "arm64" if os.uname().machine == "arm64" else "x86_64"
    out_dir = PROJECT_ROOT / "benchmarks" / "security-engine"
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / f"data_{version}_{arch}_process_enforcement.json"
    out_path.write_text(json.dumps(data, indent=2) + "\n")
    print(f"Security Engine benchmark saved to {out_path}")


def _percentile(sorted_values, percentile):
    if not sorted_values:
        raise ValueError("percentile requires at least one value")
    index = round((len(sorted_values) - 1) * percentile)
    return sorted_values[index]


def _session_db_path(service, vm):
    candidates = [
        service.tmp_dir / "sessions" / vm / "session.db",
        service.tmp_dir / "persistent" / vm / "session.db",
        *sorted((service.tmp_dir / "sessions").glob(f"{vm}*/session.db")),
    ]
    return next((path for path in candidates if path.exists()), None)


def _wait_for_security_event_count(service, vm, rule_id, expected, *, timeout=10.0):
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
                     WHERE se.event_type = 'process.exec'
                       AND step.rule_id = ?
                    """,
                    (rule_id,),
                ).fetchone()
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
        last_error = f"only {count}/{expected} process security events for {rule_id}"
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

        db_summary = _wait_for_security_event_count(svc, vm, rule_id, RUNS)
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
        _save_security_engine_benchmark(summary)

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
