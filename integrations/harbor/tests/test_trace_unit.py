"""Hermetic unit tests for `harbor_capsem._trace` (`rows.ndjson.gz` + `warc.gz` export & ATIF)."""

from __future__ import annotations

import gzip
import json
from pathlib import Path
from types import SimpleNamespace
from typing import Any

import pytest
from capsem.trace import DROPPED_SECURITY_TABLES, ROWS_STREAM_FORMAT
from capsem_integrations._controller import CommandResult
from harbor.models.task.config import EnvironmentConfig
from harbor.models.trial.paths import TrialPaths
from harbor.utils.trajectory_validator import TrajectoryValidator
from harbor_capsem import CapsemEnvironment, events_to_atif, rows_to_atif
from harbor_capsem._trace import export_and_enrich_trial_trace

_HASH_A = "blake3:" + "a" * 64
_EID_MC1 = "000000000001"


def _build_rows_and_warc(
    vm_id: str,
    *,
    include_model_calls: bool,
    single_turn_id: bool = False,
    process_name: str = "claude",
    extra_exec_rows: list[dict[str, Any]] | None = None,
) -> tuple[dict[str, list[dict[str, Any]]], bytes]:
    t1, t2 = "turn-1", ("turn-1" if single_turn_id else "turn-2")
    ex1 = {"id": 1, "timestamp": "2026-10-14T08:00:02Z", "exec_id": 1, "command": "git status"}
    ex1.update({"exit_code": 0, "stdout_preview": "On branch main", "target": "workload"})
    ex1["turn_id"] = t1
    ex2 = {"id": 2, "timestamp": "2026-10-14T08:00:06Z", "exec_id": 2, "exit_code": 0}
    ex2.update({"command": "git checkout -b fix", "stdout_preview": "Switched to a new branch"})
    ex2.update({"target": "workload", "turn_id": t2})
    aud = {"id": 1, "timestamp": "2026-10-14T08:00:02Z", "pid": 100, "ppid": 1, "uid": 0}
    aud.update({"exe": "/usr/bin/git", "argv": "git status", "turn_id": t1})
    net = {"id": 1, "timestamp": "2026-10-14T08:00:01Z", "domain": "api.anthropic.com"}
    net.update({"decision": "allowed", "status_code": 200, "turn_id": t1})
    dns = {"id": 1, "timestamp": "2026-10-14T08:00:01Z", "qname": "api.anthropic.com", "qtype": 1}
    dns.update({"qclass": 1, "rcode": 0, "decision": "allowed", "turn_id": t1})
    fs = {"id": 1, "timestamp": "2026-10-14T08:00:06Z", "action": "modify"}
    fs.update({"path": "/workspace/.git/HEAD", "turn_id": t2})
    sr = {"id": 1, "timestamp_unix_ms": 1791964802000, "event_id": "0000000000a1", "rule_id": "r1"}
    sr.update({"event_type": "process.exec", "rule_action": "allow", "detection_level": "low"})
    sr.update({"rule_json": '{"id":"r1"}', "turn_id": t1})
    sd = {"id": 1, "timestamp_unix_ms": 1791964802000, "event_id": "0000000000a2", "rule_id": "r1"}
    sd.update({"event_type": "process.exec", "stage": "rule", "actor": "engine", "turn_id": t1})
    sd.update({"previous_decision": "allow", "requested_decision": "allow"})
    sd["effective_decision"] = "allow"
    tables: dict[str, list[dict[str, Any]]] = {
        "model_calls": [],
        "model_items": [],
        "tool_calls": [],
        "tool_responses": [],
        "event_body_blobs": [],
        "exec_events": [*(extra_exec_rows or []), ex1, ex2],
        "audit_events": [aud],
        "net_events": [net],
        "dns_events": [dns],
        "fs_events": [fs],
        "security_rule_events": [sr],
        "security_decision_events": [sd],
    }
    if include_model_calls:
        mc1 = {"id": 1, "event_id": _EID_MC1, "timestamp": "2026-10-14T08:00:01Z", "method": "POST"}
        mc1.update({"provider": "anthropic", "model": "claude-sonnet-4", "path": "/v1/messages"})
        mc1.update({"process_name": process_name, "request_body_preview": "Fix git repo"})
        mc1.update({"text_content": "Running git status", "thinking_content": "Check repo first"})
        mc1.update({"input_tokens": 120, "output_tokens": 30, "estimated_cost_usd": 0.0015})
        mc1["turn_id"] = t1
        mc2 = {"id": 2, "timestamp": "2026-10-14T08:00:05Z", "provider": "anthropic"}
        mc2.update({"model": "claude-sonnet-4", "process_name": process_name, "method": "POST"})
        mc2.update({"path": "/v1/messages", "request_body_preview": "Continue", "turn_id": t2})
        mc2.update({"text_content": "Fixed the branch", "thinking_content": "All clean"})
        mc2.update({"input_tokens": 150, "output_tokens": 40, "estimated_cost_usd": 0.0020})
        mi = {"id": 1, "model_call_id": 1, "timestamp": "2026-10-14T08:00:01Z", "kind": "request"}
        mi.update({"provider": "anthropic", "model": "claude-sonnet-4", "path": "/v1/messages"})
        mi.update({"item_index": 0, "content": "Fix the broken git repository"})
        mi["content_hash"] = _HASH_A
        tc = {"id": 1, "timestamp": "2026-10-14T08:00:02Z", "model_call_id": 1, "call_index": 0}
        tc.update({"call_id": "toolu_01", "tool_name": "bash"})
        tc.update({"arguments": '{"cmd": "git status"}', "response_preview": "On branch main"})
        tc["turn_id"] = t1
        tr = {"id": 1, "model_call_id": 1, "call_id": "toolu_01"}
        tr.update({"content_preview": "On branch main", "turn_id": t1})
        tables.update({"model_calls": [mc1, mc2], "model_items": [mi]})
        tables.update({"tool_calls": [tc], "tool_responses": [tr]})
    body_bytes = b'{"messages":[{"role":"user","content":"Fix the broken git repository"}]}'
    warc_rec = (
        (
            f"WARC/1.1\r\nWARC-Type: resource\r\n"
            f"WARC-Record-ID: <urn:capsem:{vm_id}:model_calls:{_EID_MC1}:request>\r\n"
            f"WARC-Target-URI: https://api.anthropic.com/v1/messages\r\n"
            f"Content-Type: application/json\r\nContent-Length: {len(body_bytes)}\r\n\r\n"
        ).encode()
        + body_bytes
        + b"\r\n\r\n"
    )
    return tables, gzip.compress(warc_rec)


def _write_events_files(
    out_dir: Path,
    vm_id: str,
    tables: dict[str, list[dict[str, Any]]],
    warc_gz: bytes,
    *,
    include_security_events: bool = False,
) -> tuple[Path, Path]:
    out_dir.mkdir(parents=True, exist_ok=True)
    eff = {
        k: v
        for k, v in tables.items()
        if include_security_events or k not in DROPPED_SECURITY_TABLES
    }
    hdr = {"type": "header", "format": ROWS_STREAM_FORMAT, "version": 1, "vm_id": vm_id}
    hdr.update({"exported_at": "2026-10-14T08:00:07Z", "tables": list(eff.keys())})
    lines = [json.dumps(hdr)] + [
        json.dumps({"table": tbl, "row": r}) for tbl, rs in eff.items() for r in rs
    ]
    rows_p, warc_p = out_dir / f"{vm_id}.rows.ndjson.gz", out_dir / f"{vm_id}.warc.gz"
    rows_p.write_bytes(gzip.compress(("\n".join(lines) + "\n").encode("utf-8")))
    warc_p.write_bytes(warc_gz)
    return rows_p, warc_p


class _FakeTraceController:
    def __init__(
        self,
        vm_id: str,
        tables: dict[str, list[dict[str, Any]]],
        warc_gz: bytes,
        *,
        quiesce_ok: bool = True,
    ) -> None:
        self._vm_id, self.stopped_vms, self.events = vm_id, [], []
        self._tables, self._warc_gz, self._quiesce_ok = tables, warc_gz, quiesce_ok
        self._sessions: dict[str, Any] = {}

    async def start_vm(
        self, *, cpu_count: int, ram_gb: int, image: str | None = None, **kwargs: Any
    ) -> str:
        del cpu_count, ram_gb, image, kwargs

        async def _stop() -> None:
            self.events.append("vm.stop")

        async def _quiesce() -> bool:
            self.events.append("quiesce")
            return self._quiesce_ok

        async def _export_events(
            out_dir: str | Path, *, include_security_events: bool = False, **_kw: Any
        ) -> tuple[Path, Path]:
            self.events.append("export.events")
            return _write_events_files(
                Path(out_dir),
                self._vm_id,
                self._tables,
                self._warc_gz,
                include_security_events=include_security_events,
            )

        exp = SimpleNamespace(quiesce=_quiesce, events=_export_events)
        self._sessions[self._vm_id] = SimpleNamespace(stop=_stop, export=exp)
        return self._vm_id

    def get_vm_session(self, vm_id: str) -> Any:
        return self._sessions.get(vm_id)

    async def stop_vm(self, vm_id: str) -> None:
        self.events.append("stop_vm")
        self.stopped_vms.append(vm_id)

    async def exec_in_vm(self, vm_id: str, command: str, *, timeout: int = 120) -> CommandResult:
        del vm_id, timeout
        out = "/workspace\n" if command == "pwd" else ""
        return CommandResult(exit_code=0, stdout=out, stderr="")


def _write_sample_trajectory(traj_path: Path) -> None:
    traj_path.parent.mkdir(parents=True, exist_ok=True)
    tc = {"tool_call_id": "call_1", "function_name": "bash", "arguments": {"cmd": "git status"}}
    obs = {"results": [{"source_call_id": "call_1", "content": "On branch main"}]}
    s1 = {"step_id": 1, "timestamp": "2026-10-14T08:00:00Z", "source": "user", "message": "Fix"}
    s2 = {"step_id": 2, "timestamp": "2026-10-14T08:00:01Z", "source": "agent"}
    s2.update({"message": "Checking status", "tool_calls": [tc], "observation": obs, "extra": None})
    s3 = {"step_id": 3, "timestamp": "2026-10-14T08:00:05Z", "source": "agent", "message": "Fixed"}
    agent_meta = {"name": "test-agent", "version": "1.0"}
    data = {"schema_version": "ATIF-v1.7", "session_id": "sess-1", "agent": agent_meta}
    data["steps"] = [s1, s2, s3]
    traj_path.write_text(json.dumps(data, indent=2), encoding="utf-8")


@pytest.mark.parametrize(
    ("quiesce_ok", "expected_events"),
    [
        (True, ["quiesce", "export.events", "stop_vm"]),
        (False, ["quiesce", "vm.stop", "export.events", "stop_vm"]),
    ],
)
async def test_turn_id_enrichment_warc_export_and_drop_security_tables(
    tmp_path: Path, quiesce_ok: bool, expected_events: list[str]
) -> None:
    vm_id = "vm-cli-1"
    tables, warc_gz = _build_rows_and_warc(vm_id, include_model_calls=True)
    trial_paths = TrialPaths(trial_dir=tmp_path / "trial")
    traj_path = trial_paths.agent_dir / "trajectory.json"
    _write_sample_trajectory(traj_path)

    env_dir = tmp_path / "env"
    env_dir.mkdir()
    ctrl: Any = _FakeTraceController(vm_id, tables, warc_gz, quiesce_ok=quiesce_ok)
    tcfg = EnvironmentConfig(docker_image="ubuntu:24.04")
    env = CapsemEnvironment(
        env_dir, vm_id, f"{vm_id}__1", trial_paths, tcfg, export_trace=True, controller=ctrl
    )
    await env.start(force_build=False)
    await env.stop(delete=True)
    assert ctrl.stopped_vms == [vm_id] and ctrl.events == expected_events

    exported_rows = trial_paths.trial_dir / f"{vm_id}.rows.ndjson.gz"
    exported_warc = trial_paths.trial_dir / f"{vm_id}.warc.gz"
    assert exported_rows.is_file() and exported_warc.is_file()
    hdr = json.loads(gzip.decompress(exported_rows.read_bytes()).decode("utf-8").splitlines()[0])
    assert not (set(DROPPED_SECURITY_TABLES) & set(hdr["tables"]))

    enriched = json.loads(traj_path.read_text(encoding="utf-8"))
    assert TrajectoryValidator().validate(traj_path, validate_images=False) is True
    summary = enriched["extra"]["capsem_summary"]
    assert (summary["join_mode"], summary["model_calls_count"]) == ("turn_id", 2)
    fm = enriched["final_metrics"]
    assert (fm["total_prompt_tokens"], fm["total_completion_tokens"]) == (270, 70)
    assert summary["rows_ndjson_gz"] == f"{vm_id}.rows.ndjson.gz"
    assert summary["warc_gz"] == f"{vm_id}.warc.gz"


async def test_timestamp_window_join_and_synthesize_atif(tmp_path: Path) -> None:
    vm_id = "vm-host-1"
    tables, warc_gz = _build_rows_and_warc(vm_id, include_model_calls=False)
    trial_paths = TrialPaths(trial_dir=tmp_path / "trial_host")
    traj_path = trial_paths.agent_dir / "trajectory.json"
    _write_sample_trajectory(traj_path)
    env_dir = tmp_path / "env_host"
    env_dir.mkdir()
    ctrl: Any = _FakeTraceController(vm_id, tables, warc_gz)
    env = CapsemEnvironment(
        env_dir,
        vm_id,
        f"{vm_id}__1",
        trial_paths,
        EnvironmentConfig(docker_image="ubuntu:24.04"),
        export_trace="true",
        include_security_events="true",
        controller=ctrl,
    )
    await env.start(force_build=False)
    await env.stop(delete=True)
    exported_rows = trial_paths.trial_dir / f"{vm_id}.rows.ndjson.gz"
    hdr = json.loads(gzip.decompress(exported_rows.read_bytes()).decode("utf-8").splitlines()[0])
    assert {"security_rule_events", "security_decision_events"} <= set(hdr["tables"])

    enriched = json.loads(traj_path.read_text(encoding="utf-8"))
    assert TrajectoryValidator().validate(traj_path, validate_images=False) is True
    assert enriched["extra"]["capsem_summary"]["join_mode"] == "timestamp_window"
    s2_events = enriched["steps"][1]["extra"]["capsem_events"]
    s3_events = enriched["steps"][2]["extra"]["capsem_events"]
    assert [e["command"] for e in s2_events["exec_events"]] == ["git status"]
    assert [e["command"] for e in s3_events["exec_events"]] == ["git checkout -b fix"]

    synth_vm = "vm-synth-1"
    ver_row = {"id": 0, "timestamp": "2026-10-14T08:00:00Z", "exec_id": 0, "exit_code": 0}
    ver_row.update({"command": "aider --version", "stdout_preview": "aider 0.86.2\n"})
    ver_row["target"] = "workload"
    s_tables, s_warc = _build_rows_and_warc(
        synth_vm,
        include_model_calls=True,
        single_turn_id=True,
        process_name="python",
        extra_exec_rows=[ver_row],
    )
    synth_paths = TrialPaths(trial_dir=tmp_path / "trial_synth")
    synth_paths.trial_dir.mkdir(parents=True, exist_ok=True)
    synth_paths.lock_path.write_text('{"agent": {"name": "aider"}}', "utf-8")
    rows_p, _ = _write_events_files(synth_paths.trial_dir, synth_vm, s_tables, s_warc)
    out_rows, _ = await export_and_enrich_trial_trace(vm_id=synth_vm, trial_paths=synth_paths)
    assert out_rows == rows_p
    synth_traj = json.loads((synth_paths.agent_dir / "trajectory.json").read_text(encoding="utf-8"))
    exp_agent = {"name": "aider", "version": "0.86.2", "model_name": "claude-sonnet-4"}
    assert synth_traj["agent"] == exp_agent and events_to_atif is rows_to_atif
