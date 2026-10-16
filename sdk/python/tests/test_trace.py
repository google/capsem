"""Hermetic unit tests for `capsem.trace` over `rows.ndjson.gz` and `.warc.gz`."""

from __future__ import annotations

import base64
import datetime as dt
import gzip
import json
import sys
from pathlib import Path
from types import SimpleNamespace
from typing import Any, cast

import capsem
import pytest
from capsem import trace

_HASH_A, _HASH_B = "blake3:" + "a" * 64, "blake3:" + "b" * 64
_CRED_REF = "credential:blake3:" + "c" * 64


def make_warc_record_gz(headers: list[tuple[str, str]], payload: bytes = b"") -> bytes:
    lines = ["WARC/1.1", *[f"{k}: {v}" for k, v in headers], f"Content-Length: {len(payload)}", "", ""]
    return gzip.compress("\r\n".join(lines).encode("utf-8") + payload + b"\r\n\r\n")


def make_resource_warc_gz(
    vm_id: str, records: list[tuple[str, str, str, bytes, str, bool]]
) -> bytes:
    chunks = [
        make_warc_record_gz(
            [
                ("WARC-Type", "warcinfo"),
                ("WARC-Date", "2026-10-14T08:00:00Z"),
                ("WARC-Record-ID", f"<urn:capsem:warcinfo:{vm_id}>"),
                ("Content-Type", "application/warc-fields"),
            ],
            b"software: capsem\r\nformat: WARC File Format 1.1\r\n",
        )
    ]
    for table, eid, direction, body, ctype, trunc in records:
        hdrs = [
            ("WARC-Type", "resource"),
            ("WARC-Date", "2026-10-14T08:00:01Z"),
            ("WARC-Record-ID", f"<urn:capsem:{vm_id}:{table}:{eid}:{direction}>"),
            ("WARC-Target-URI", f"capsem://session/{vm_id}/{table}/{eid}/{direction}"),
            ("WARC-Payload-Digest", _HASH_A),
            ("Content-Type", ctype),
        ]
        if trunc:
            hdrs.append(("WARC-Truncated", "length"))
        chunks.append(make_warc_record_gz(hdrs, body))
    return b"".join(chunks)


def populate_session_exports(
    base_dir: Path,
    vm_id: str,
    *,
    with_models: bool = True,
    with_exec: bool = True,
    single_turn: bool = False,
    include_security: bool = True,
) -> tuple[Path, Path]:
    base_dir.mkdir(parents=True, exist_ok=True)
    tid2 = "turn-1" if single_turn else "turn-2"
    ts1, ts2 = "2026-10-14T08:00:01Z", "2026-10-14T08:00:02Z"
    tables = list(trace.ALL_EXPORT_TABLES if include_security else trace.DEFAULT_EXPORT_TABLES)
    lines: list[dict[str, Any]] = [
        {"type": "header", "format": "capsem.rows.v1", "format_version": 1, "vm_id": vm_id, "exported_at": ts1, "tables": tables},
        {"table": "fs_events", "row": {"id": 1, "event_id": "000000000051", "timestamp": "2026-10-14T08:00:03Z", "action": "modified", "path": "/repo/a", "kind": "file", "turn_id": "turn-1"}},
        {"table": "net_events", "row": {"id": 1, "event_id": "000000000061", "timestamp": ts1, "domain": "generativelanguage.googleapis.com", "decision": "allowed", "status_code": 200, "request_body_preview": '{"net":1}', "turn_id": "turn-1"}},
        {"table": "dns_events", "row": {"id": 1, "event_id": "000000000071", "timestamp": ts1, "qname": "api.google.com", "qtype": 1, "qclass": 1, "rcode": 0, "decision": "allowed", "turn_id": "turn-1"}},
        {"table": "audit_events", "row": {"id": 1, "event_id": "000000000081", "timestamp": ts2, "pid": 10, "ppid": 1, "uid": 0, "exe": "/bin/git", "argv": "git status", "turn_id": "turn-1"}},
        {"table": "substitution_events", "row": {"id": 1, "event_id": "000000000091", "timestamp": "2026-10-14T08:00:00Z", "material_class": "api_key", "source": "env", "algorithm": "blake3", "substitution_ref": _CRED_REF, "outcome": "brokered", "provider": "google", "turn_id": "trace-sub"}},
    ]
    if include_security:
        lines.extend([
            {"table": "security_rule_events", "row": {"id": 1, "timestamp_unix_ms": 1791964802000, "event_id": "0000000000a1", "event_type": "process.exec", "rule_id": "r1", "rule_action": "allow", "detection_level": "low", "rule_json": '{"id":"r1"}', "turn_id": "turn-1"}},
            {"table": "security_decision_events", "row": {"id": 1, "timestamp_unix_ms": 1791964802000, "event_id": "0000000000a2", "event_type": "process.exec", "stage": "rule", "actor": "engine", "rule_id": "r1", "previous_decision": "allow", "requested_decision": "allow", "effective_decision": "allow", "turn_id": "turn-1"}},
            {"table": "security_ask_events", "row": {"id": 1, "timestamp_unix_ms": 1791964802000, "ask_id": "0000000000a3", "event_id": "0000000000a4", "event_type": "process.exec", "rule_id": "r1", "rule_name": "rule1", "status": "approved", "rule_json": '{"id":"r1"}', "turn_id": "turn-1"}},
        ])
    blobs: list[dict[str, Any]] = [
        {"id": 1, "event_id": "0000000000a1", "event_type": "security.rule", "source_table": "security_rule_events", "direction": "payload", "content_type": "application/vnd.capsem.security+msgpack", "original_bytes": 36, "stored_bytes": 36, "truncated": 0, "body_hash": _HASH_B, "turn_id": "turn-1"}
    ]
    if with_models:
        lines.extend([
            {"table": "model_calls", "row": {"id": 1, "event_id": "000000000001", "timestamp": ts1, "provider": "google", "protocol": "google", "model": "gemini-2.5-pro", "process_name": "gemini-cli", "method": "POST", "path": "/v1/m", "system_prompt_preview": "You are helpful.", "messages_count": 1, "tools_count": 1, "text_content": None, "thinking_content": None, "stop_reason": "tool_use", "input_tokens": 120, "output_tokens": 40, "usage_details": '{"cached_tokens":20}', "duration_ms": 450, "estimated_cost_usd": 0.002, "request_body_preview": '{"req":', "turn_id": "turn-1"}},
            {"table": "model_calls", "row": {"id": 2, "event_id": "000000000002", "timestamp": "2026-10-14T08:00:05Z", "provider": "google", "protocol": "google", "model": "gemini-2.5-pro", "process_name": "gemini-cli", "method": "POST", "path": "/v1/m", "system_prompt_preview": None, "messages_count": 3, "tools_count": 1, "text_content": "Fixed.", "thinking_content": "Done thinking.", "stop_reason": "end_turn", "input_tokens": 160, "output_tokens": 20, "usage_details": "not-json", "duration_ms": 300, "estimated_cost_usd": 0.001, "request_body_preview": None, "turn_id": tid2}},
            {"table": "model_items", "row": {"id": 1, "event_id": "000000000011", "model_call_id": 1, "timestamp": ts1, "provider": "google", "model": "m", "path": "/v1/m", "trace_id": "tr-1", "turn_id": "turn-1", "kind": "reasoning", "item_index": 0, "call_id": "", "content": "Check git status", "content_hash": _HASH_A}},
            {"table": "model_items", "row": {"id": 2, "event_id": "000000000012", "model_call_id": 1, "timestamp": ts1, "provider": "google", "model": "m", "path": "/v1/m", "trace_id": "tr-1", "turn_id": "turn-1", "kind": "response", "item_index": 1, "call_id": "", "content": "Running git status", "content_hash": _HASH_B}},
            {"table": "tool_calls", "row": {"id": 1, "event_id": "000000000021", "timestamp": ts2, "model_call_id": 1, "provider": "google", "status": "responded", "call_index": 0, "call_id": "call-1", "tool_name": "bash", "arguments": '{"cmd":"git status"}', "response_preview": "On branch", "origin": "native", "transport": "http", "turn_id": "turn-1"}},
            {"table": "tool_responses", "row": {"id": 1, "event_id": "000000000031", "model_call_id": 2, "call_id": "call-1", "content_preview": "On branch", "is_error": 0, "turn_id": "turn-1"}},
        ])
        blobs.extend([
            {"id": 2, "event_id": "000000000001", "event_type": "model.call", "source_table": "model_calls", "direction": "request", "content_type": "application/json", "original_bytes": 64, "stored_bytes": 50, "truncated": 1, "body_hash": _HASH_A, "turn_id": "turn-1"},
            {"id": 3, "event_id": "000000000001", "event_type": "model.call", "source_table": "model_calls", "direction": "response", "content_type": "application/json", "original_bytes": 64, "stored_bytes": 50, "truncated": 1, "body_hash": _HASH_B, "turn_id": "turn-1"},
            {"id": 4, "event_id": "000000000031", "event_type": "model.call", "source_table": "tool_responses", "direction": "response", "content_type": "text/plain", "original_bytes": 32, "stored_bytes": 24, "truncated": 1, "body_hash": _HASH_A, "turn_id": "turn-1"},
        ])
    if with_exec:
        lines.extend([
            {"table": "exec_events", "row": {"id": 1, "event_id": "000000000041", "timestamp": ts2, "exec_id": 1, "command": "git status", "exit_code": 0, "duration_ms": 12, "stdout_preview": "On branch", "stderr_preview": "", "target": "workload", "turn_id": "trace-exec-1", "process_name": "bash"}},
            {"table": "exec_events", "row": {"id": 2, "event_id": "000000000042", "timestamp": "2026-10-14T08:00:06Z", "exec_id": 2, "command": "git checkout -b fix", "exit_code": 0, "duration_ms": 15, "stdout_preview": "Switched", "stderr_preview": "", "target": "workload", "turn_id": "trace-exec-2", "process_name": "bash"}},
        ])
        blobs.append({"id": 5, "event_id": "000000000041", "event_type": "process.exec_complete", "source_table": "exec_events", "direction": "stdout", "content_type": "text/plain", "original_bytes": 20, "stored_bytes": 20, "truncated": 0, "body_hash": _HASH_A, "turn_id": "trace-exec-1"})
    for b in blobs:
        lines.append({"table": "event_body_blobs", "row": b})

    rows_path, warc_path = base_dir / f"{vm_id}.rows.ndjson.gz", base_dir / f"{vm_id}.warc.gz"
    rows_path.write_bytes(gzip.compress(("\n".join(json.dumps(item) for item in lines) + "\n").encode("utf-8")))
    req_json = json.dumps({"system": "Full system prompt from archive", "messages": [{"role": "user", "content": "hi"}]}).encode("utf-8")
    resp_json = json.dumps({"candidates": [{"content": "Full response from archive"}]}).encode("utf-8")
    sec_json = json.dumps({"event_type": "process.exec", "rule": "r1"}).encode("utf-8")
    warc_path.write_bytes(
        make_resource_warc_gz(
            vm_id,
            [
                ("model_calls", "000000000001", "request", req_json, "application/json", False),
                ("model_calls", "000000000001", "response", resp_json, "application/json", False),
                ("tool_responses", "000000000031", "response", b"On branch main (full hydrated tool output)", "text/plain", False),
                ("exec_events", "000000000041", "stdout", b"On branch main (full stdout)", "text/plain", False),
                ("security_rule_events", "0000000000a1", "payload", sec_json, "application/json", False),
            ],
        )
    )
    return rows_path, warc_path


def test_timestamp_flags_rows_and_warc_reader(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    assert trace.parse_iso_datetime(None) is None and trace.parse_iso_datetime("   ") is None
    assert trace.parse_iso_datetime("not-a-timestamp") is None
    naive = dt.datetime.fromisoformat("2026-10-14T08:00:00")
    aware = dt.datetime(2026, 10, 14, 10, 0, 0, tzinfo=dt.timezone(dt.timedelta(hours=2)))
    utc_ref = dt.datetime(2026, 10, 14, 8, 0, 0, tzinfo=dt.UTC)
    assert trace.parse_iso_datetime(naive) == utc_ref and trace.parse_iso_datetime(aware) == utc_ref
    assert trace.parse_iso_datetime("2026-10-14T08:00:00Z") == utc_ref
    monkeypatch.delenv("CAPSEM_TRACE_INCLUDE_SECURITY", raising=False)
    assert not trace.resolve_include_security_events()
    assert trace.resolve_include_security_events(include_security_events=True)
    monkeypatch.setenv("CAPSEM_TRACE_INCLUDE_SECURITY", "1")
    assert trace.resolve_include_security_events()
    assert not trace.resolve_include_security_events(include_security_events=False)

    rows_path, warc_path = populate_session_exports(tmp_path, "vm-1")
    assert trace.has_session_events(rows_path) and not trace.has_session_events(tmp_path / "no.ndjson.gz")
    assert trace.resolve_warc_path(rows_path) == warc_path
    meta_path = trace.write_export_meta(
        tmp_path, "vm-1", rows_path=rows_path, warc_path=warc_path, include_security_events=False, labels={"task": "t1"}
    )
    assert json.loads(meta_path.read_text())["labels"] == {"task": "t1"}
    assert trace.load_rows_tables(tmp_path / "no.ndjson.gz", ("exec_events",)) == {"exec_events": []}
    reader = trace.WarcBodyReader(warc_path)
    assert len(reader.get_event_bodies("000000000001")) == 2
    assert reader.get_event_bodies("missing") == ()


def test_synthesize_exec_only_and_enrich_atif(tmp_path: Path) -> None:
    rows1, warc1 = populate_session_exports(tmp_path, "vm-turn", with_models=True, single_turn=True)
    synth = trace.rows_to_atif(rows1, vm_id="vm-turn", warc_path=warc1, output_path=tmp_path / "synth.json")
    assert trace.validate_atif_trajectory(synth) == []
    assert synth["agent"]["version"] == capsem.__version__ and "capsem" not in synth["extra"]
    summary = synth["extra"]["capsem_summary"]
    assert summary["rows_ndjson_gz"] == "vm-turn.rows.ndjson.gz"
    assert summary["warc_gz"] == "vm-turn.warc.gz"
    assert summary["dropped_security_tables"] is True and summary["exec_events_count"] == 2
    assert synth["steps"][0]["source"] == "system"
    assert synth["steps"][0]["message"] == "Full system prompt from archive"
    assert synth["steps"][2]["extra"]["capsem_events"]["join_mode"] == "timestamp_window"
    assert [len(synth["steps"][i]["extra"]["capsem_events"]["exec_events"]) for i in (2, 3)] == [1, 1]

    rows_exec, warc_exec = populate_session_exports(tmp_path, "vm-exec", with_models=False, with_exec=True)
    exec_traj = trace.rows_to_atif(rows_exec, vm_id="vm-exec", warc_path=warc_exec, bodies="previews")
    assert trace.validate_atif_trajectory(exec_traj) == []
    assert exec_traj["extra"]["capsem_summary"]["join_mode"] == "exec_only"
    assert exec_traj["steps"][1]["llm_call_count"] == 0
    assert exec_traj["steps"][1]["message"] == "git status"
    assert exec_traj["steps"][1]["extra"]["capsem_events"]["audit_events"][0]["argv"] == "git status"

    rows_empty, _ = populate_session_exports(tmp_path, "vm-empty", with_models=False, with_exec=False)
    assert not trace.has_session_events(rows_empty)
    with pytest.raises(ValueError, match="no model_calls or exec_events"):
        trace.rows_to_atif(rows_empty, vm_id="vm-empty")
    with pytest.raises(ValueError, match="Invalid bodies mode"):
        trace.rows_to_atif(rows_exec, vm_id="vm-exec", bodies=cast(Any, "bad"))

    traj_ord: dict[str, Any] = {
        "schema_version": "ATIF-v1.8", "session_id": "s-ord",
        "agent": {"name": "a", "version": "1"},
        "steps": [
            {"step_id": 1, "source": "agent", "message": [{"text": "m1"}], "extra": None},
            {"step_id": 2, "source": "agent", "message": "m2"},
        ],
    }
    assert trace.validate_atif_trajectory(traj_ord) == []
    res_ord = trace.enrich_atif_trajectory(
        traj_ord, rows1, vm_id="vm-turn", warc_path=warc1, output_path=tmp_path / "ord.json"
    )
    assert res_ord["steps"][0]["extra"]["capsem_events"]["join_mode"] == "ordinal_fallback"
    assert (tmp_path / "ord.json").is_file() and not trace.enrich_steps_by_turn_id([], [], {})
    trace.enrich_steps_by_timestamp_window([], {})


def test_step_builder_edge_cases_and_hydration_heuristics(monkeypatch: pytest.MonkeyPatch) -> None:
    def _tc(mc: int | None, cid: str, fn: str, args: str = "{}") -> dict[str, Any]:
        return {"model_call_id": mc, "call_id": cid, "tool_name": fn, "arguments": args}

    def _tr(mc: int, cid: str, prev: str, err: int = 0) -> dict[str, Any]:
        return {"model_call_id": mc, "call_id": cid, "content_preview": prev, "is_error": err}

    u_msgs = [{"role": "user", "content": "x"}, {"role": "user", "content": "Responses string prompt"}]
    mcalls = [
        {"id": 1, "timestamp": "2026-10-14T08:00:01Z",
         "system_prompt_preview": "sys", "request_body_preview": '{"text": "hello \\u00'},
        {"id": 2, "timestamp": "2026-10-14T08:00:02Z",
         "request_body": json.dumps({"input": "f", "messages": u_msgs})},
    ]
    items = [
        {"model_call_id": None, "kind": "response", "content": "orphan"},
        {"model_call_id": "bad", "kind": "response", "content": "bad"},
        {"model_call_id": 1, "kind": "response", "content": "ok"},
    ]
    tcalls = [
        _tc(None, "orphan", "bash"), _tc(1, "call_1_0", "read", "not-json"),
        _tc(1, "call_1_1", "read_file"), _tc(2, "c_ord_0", "alpha"), _tc(2, "c_ord_1", "beta"),
    ]
    tresp = [
        _tr(2, "read_file_99_1", "f-c"), _tr(2, "read_99_0", "r-c"),
        _tr(2, "r0", "first-ordinal"), _tr(2, "r1", "second-ordinal", 1),
    ]
    steps = trace.build_steps_from_session_tables(mcalls, items, tcalls, tresp)
    assert steps[0]["message"] == "sys" and steps[1]["message"] == "hello"
    assert [r["content"] for r in steps[2]["observation"]["results"]] == ["r-c", "f-c"]
    assert [r["content"] for r in steps[3]["observation"]["results"]] == ["first-ordinal", "second-ordinal"]
    assert trace.build_steps_from_session_tables([mcalls[1]], [], [], [])[0]["message"] == "Responses string prompt"

    def _blob(eid: str, tbl: str, d: str, b: int, tr: int = 0) -> dict[str, Any]:
        return {"event_id": eid, "source_table": tbl, "direction": d, "stored_bytes": b, "truncated": tr}

    tables: dict[str, list[dict[str, Any]]] = {
        "model_calls": [
            {"id": 1, "event_id": "000000000001", "system_prompt_preview": "s", "request_body_preview": '{"full_request": 1}'},
            {"id": 2, "event_id": "000000000002", "request_body_preview": '{"trunc":'},
        ],
        "tool_calls": [{"id": 1, "event_id": "000000000021", "arguments": '{"a": 1}'}],
        "tool_responses": [], "exec_events": [],
        "security_rule_events": [{"id": 1, "event_id": "0000000000a1"}],
        "event_body_blobs": [
            _blob("000000000001", "model_calls", "request", 19),
            _blob("000000000001", "model_calls", "response", 500),
            _blob("000000000002", "model_calls", "request", 100, 1),
            _blob("000000000021", "tool_calls", "request", 8),
            _blob("0000000000a1", "security_rule_events", "payload", 16),
        ],
    }
    assert trace.candidate_body_event_ids(tables, full=False) == []
    assert trace.candidate_body_event_ids(tables, full=True) == [
        "000000000001", "000000000002", "000000000021", "0000000000a1"
    ]

    class _DummyReader:
        def get_event_bodies(self, event_id: str) -> list[dict[str, Any]]:
            return [{"event_id": event_id, "source_table": "tool_calls", "direction": "request", "encoding": "utf-8", "content": '{"hydrated": true}'}]

    trace.hydrate_tables_from_bodies(tables, _DummyReader(), full=True)
    assert tables["tool_calls"][0]["arguments"] == '{"hydrated": true}'
    raw_bin = b"\x81\xa4rule\xa2r1"
    b64 = base64.b64encode(raw_bin).decode("ascii")
    env = {"encoding": "base64", "data": b64, "content_type": "application/msgpack"}
    assert trace.decode_security_payload(env) == env
    assert trace.decode_security_payload(raw_bin) == {"encoding": "base64", "data": b64}
    assert trace.decode_security_payload({"encoding": "utf-8", "data": '{"k": 1}'}) == {"k": 1}
    assert trace.decode_security_payload({"encoding": "utf-8", "data": "plain"}) == "plain"
    monkeypatch.setitem(sys.modules, "msgpack", SimpleNamespace(unpackb=lambda data, **_kw: {"rule": "r1", "len": len(data)}))
    assert trace.decode_security_payload(raw_bin) == {"rule": "r1", "len": len(raw_bin)}
    with pytest.raises(ValueError, match="12 lowercase hex"):
        trace.validate_event_id("../bad")
    assert trace.validate_atif_trajectory({"schema_version": "bad", "session_id": ""}) != []
