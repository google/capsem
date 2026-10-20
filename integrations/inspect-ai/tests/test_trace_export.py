"""Hermetic unit tests for `inspect_capsem.trace_export` and `inspect_capsem._eval_loader`."""

from __future__ import annotations

import asyncio
import contextlib
import gzip
import json
import sys
from pathlib import Path
from types import SimpleNamespace
from typing import Any

import capsem
import capsem.models
import pytest
from capsem.trace import ROWS_STREAM_FORMAT, quiesce_vm_for_export
from inspect_capsem._eval_loader import (
    discover_eval_files,
    export_sample_vm_events,
    format_iso_utc,
    load_eval_log_dict,
    match_sample_db,
    parse_iso_epoch,
    resolve_sample_refs_and_attachments,
)
from inspect_capsem._trace_convert import (
    _is_errored_model_event,
    _model_event_message_id,
    _pair_model_event,
    _reasoning_of,
    _text_of,
)
from inspect_capsem.sandbox import CapsemSandboxEnvironment
from inspect_capsem.trace_export import (
    convert_eval_log,
    export_traces,
    main,
    run_with_signal_forwarding,
    validate_trajectory_file,
)


def _write_rows_ndjson_gz(path: Path, vm_id: str, tables: dict[str, list[dict[str, Any]]]) -> Path:
    hdr = {"type": "header", "format": ROWS_STREAM_FORMAT, "version": 1, "vm_id": vm_id}
    hdr.update({"exported_at": "2026-10-14T05:00:05Z", "tables": list(tables.keys())})
    lines = [json.dumps(hdr)] + [
        json.dumps({"table": tbl, "row": r}) for tbl, rs in tables.items() for r in rs
    ]
    path.write_bytes(gzip.compress(("\n".join(lines) + "\n").encode("utf-8")))
    return path


def _stub_eval_logs(monkeypatch: pytest.MonkeyPatch, logs: dict[Path, dict[str, Any]]) -> None:
    resolved = {p.resolve(): data for p, data in logs.items()}
    for p in logs:
        p.write_text("{}", encoding="utf-8")

    def _fake_read(log_file: str, **_kwargs: Any) -> Any:
        payload = resolved[Path(log_file).resolve()]
        return SimpleNamespace(model_dump=lambda mode=None: json.loads(json.dumps(payload)))

    monkeypatch.setattr("inspect_capsem._eval_loader.read_eval_log", _fake_read)


def test_resolve_sample_refs_and_attachments() -> None:
    call_obj = {"call_refs": [[0, 1]], "call_key": "contents", "request": {"s": "attachment://a1"}}
    events_data = {
        "messages": [{"role": "system", "content": "attachment://a1"}],
        "calls": [{"role": "user", "parts": [{"text": "hello"}]}],
    }
    sample: dict[str, Any] = {
        "attachments": {"a1": "resolved-sys", "a2": "resolved-out"},
        "events_data": events_data,
        "events": [{"event": "model", "input_refs": [[0, 1]], "call": call_obj}],
        "messages": [{"role": "tool", "content": "attachment://a2"}],
    }
    resolved = resolve_sample_refs_and_attachments(sample)
    assert resolved["events"][0]["input"][0]["content"] == "resolved-sys"
    assert resolved["events"][0]["call"]["request"]["contents"][0]["parts"][0]["text"] == "hello"
    assert resolved["messages"][0]["content"] == "resolved-out"
    assert resolve_sample_refs_and_attachments({"messages": []}) == {"messages": []}


def test_load_eval_log_dict_and_discover_eval_files(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    with pytest.raises(FileNotFoundError, match="Inspect eval log not found"):
        load_eval_log_dict(tmp_path / "missing.eval")
    eval_z = tmp_path / "journal.eval"
    eval_z.write_text("{}", encoding="utf-8")
    payload = {"eval": {"run_id": "r1"}, "samples": [{"id": "s1", "messages": []}]}
    fake_ok = SimpleNamespace(model_dump_json=lambda: json.dumps(payload))
    monkeypatch.setattr("inspect_capsem._eval_loader.read_eval_log", lambda *_a, **_kw: fake_ok)
    loaded = load_eval_log_dict(eval_z)
    assert loaded["eval"]["run_id"] == "r1" and discover_eval_files([tmp_path, eval_z]) == [eval_z]
    fake_bad = SimpleNamespace(model_dump=lambda mode=None: ["not-a-dict"])
    monkeypatch.setattr("inspect_capsem._eval_loader.read_eval_log", lambda *_a, **_kw: fake_bad)
    with pytest.raises(ValueError, match="Expected dict from read_eval_log"):
        load_eval_log_dict(eval_z)


@pytest.mark.parametrize(
    ("quiesce_ok", "expected_prefix"), [(True, ["quiesce"]), (False, ["quiesce", "vm.stop"])]
)
async def test_sandbox_cleanup_exports_session_events(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, quiesce_ok: bool, expected_prefix: list[str]
) -> None:
    export_dir = tmp_path / "traces"
    monkeypatch.setenv("CAPSEM_TRACE_EXPORT_DIR", str(export_dir))
    calls: list[str] = []
    exported: dict[str, Any] = {}

    async def fake_quiesce() -> bool:
        calls.append("quiesce")
        return quiesce_ok

    async def fake_vm_stop() -> Any:
        calls.append("vm.stop")
        return capsem.models.StopResponse(success=True, persistent=False)

    async def fake_events(out: Any, *, labels: Any = None, write_meta: bool = False) -> Any:
        calls.append("export.events")
        exported.update({"out": str(out), "labels": dict(labels or {}), "meta": write_meta})

    exp_ns = SimpleNamespace(quiesce=fake_quiesce, events=fake_events)
    vm_obj = SimpleNamespace(id="vm-ctrl-1", stop=fake_vm_stop, export=exp_ns)

    class _Ctrl:
        def get_vm_session(self, vm_id: str) -> Any:
            assert vm_id == "vm-ctrl-1"
            return vm_obj

        async def stop_vm(self, vm_id: str) -> None:
            calls.append(f"stop_vm:{vm_id}")

        async def close(self) -> None:
            calls.append("close")

    ctrl: Any = _Ctrl()
    env = CapsemSandboxEnvironment("vm-ctrl-1", ctrl, owns_controller=True, task_name="é" * 200)
    await env.cleanup()
    assert calls == [*expected_prefix, "export.events", "stop_vm:vm-ctrl-1", "close"]
    expected_labels = {"managed-by": "inspect-capsem", "inspect-capsem-task": "é" * 127}
    assert exported == {"out": str(export_dir), "labels": expected_labels, "meta": True}
    await export_sample_vm_events(SimpleNamespace(), "missing-vm")
    assert parse_iso_epoch("1970-01-01T00:00:10Z") == 10.0 and format_iso_utc(None) is None
    assert format_iso_utc(0.0) == "1970-01-01T00:00:00Z"
    db_a, db_b = tmp_path / "a.rows.ndjson.gz", tmp_path / "b.rows.ndjson.gz"
    cache = {db_a: {"exec_events": [(5.0, {"command": "pytest"})]}, db_b: {"exec_events": []}}
    assert match_sample_db(cache, 1.0, 9.0, {"pytest"}) == db_a
    for err in (RuntimeError("export boom"), asyncio.CancelledError()):
        calls.clear()

        async def _fail_export(*_a: Any, _e: BaseException = err, **_kw: Any) -> None:
            raise _e

        monkeypatch.setattr("inspect_capsem.sandbox.export_sample_vm_events", _fail_export)
        env2 = CapsemSandboxEnvironment("vm-ctrl-1", ctrl, owns_controller=True, task_name="t1")
        is_cancel = isinstance(err, asyncio.CancelledError)
        with pytest.raises(asyncio.CancelledError) if is_cancel else contextlib.nullcontext():
            await env2.cleanup()
        assert calls == ["stop_vm:vm-ctrl-1", "close"]


@pytest.mark.parametrize(
    ("status", "error", "expected_stops"),
    [(400, "persist error", []), (409, "persist error", []), (400, "VM is already persistent", ["stop"])],
)
async def test_quiesce_vm_for_export_errors(status: int, error: str, expected_stops: list[str]) -> None:
    stopped: list[str] = []

    async def _fail_persist(_name: str) -> None:
        raise capsem.HttpError(status, f'{{"error":"{error}"}}')

    async def _rec_stop() -> None:
        stopped.append("stop")

    await quiesce_vm_for_export(SimpleNamespace(persist=_fail_persist, stop=_rec_stop), "vm-x")
    assert stopped == expected_stops


_PARTS = [{"type": "reasoning", "reasoning": "think"}, {"type": "text", "text": "act"}]


@pytest.mark.parametrize(
    ("val", "expected_text", "expected_reasoning"),
    [(None, "", None), ("plain", "plain", None), (123, "123", None), (_PARTS, "act", "think")],
)
def test_text_and_reasoning_extractors(
    val: Any, expected_text: str, expected_reasoning: str | None
) -> None:
    assert _text_of(val) == expected_text and _reasoning_of(val) == expected_reasoning


@pytest.mark.parametrize(
    ("event", "is_err", "msg_id"),
    [
        ({"error": "rate limit"}, True, None),
        ({"output": {"error": "bad"}}, True, None),
        ({"output": {"choices": [{"stop_reason": "error"}]}}, True, None),
        ({"output": "bad"}, False, None),
        ({"output": {"message": {"id": "m-1"}}}, False, "m-1"),
        ({"output": {"choices": [{"message": {"id": "m-2"}}]}}, False, "m-2"),
    ],
)
def test_model_event_predicates(event: dict[str, Any], is_err: bool, msg_id: str | None) -> None:
    assert _is_errored_model_event(event) is is_err and _model_event_message_id(event) == msg_id


def test_pair_model_event_and_validate_invalid(tmp_path: Path) -> None:
    mevs = [{"output": {"message": {"id": "m2"}}}, {"output": {}}]
    matched, idx = _pair_model_event("m1", mevs, {"m2": (0, mevs[0])}, set(), 0)
    assert matched == mevs[1] and idx == 2
    for bad_txt in ("{not-json", "[]"):
        bad_file = tmp_path / "bad.json"
        bad_file.write_text(bad_txt, encoding="utf-8")
        assert validate_trajectory_file(bad_file) != []


def test_export_traces_merges_rows_and_pairs_by_message_id(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    traces_dir, eval_file = tmp_path / "traces", tmp_path / "bench.eval"
    traces_dir.mkdir()
    u1 = {"input_tokens": 42, "output_tokens": 15, "input_tokens_cache_read": 10}
    u1["reasoning_tokens"] = 5
    c1 = {"call_refs": [[0, 1]], "call_key": "contents", "request": {"sys": ["attachment://att_s"]}}
    c1["response"] = {"candidates": [{"content": {"parts": [{"text": "Step 1"}]}}]}
    m1_ev = {"event": "model", "timestamp": "2026-10-14T05:00:01Z", "input_refs": [[0, 1]]}
    m1_ev.update({"output": {"message": {"id": "msg-1"}, "usage": u1}, "call": c1})
    m_err = {"event": "model", "timestamp": "2026-10-14T05:00:02Z", "error": "HTTP 429"}
    m_err["output"] = {"choices": [{"message": {"id": "err"}, "stop_reason": "error"}]}
    m2_out = {"choices": [{"message": {"id": "msg-2"}, "stop_reason": "stop"}]}
    m2_out["usage"] = {"input_tokens": 50, "output_tokens": 10}
    m2_ev = {"event": "model", "timestamp": "2026-10-14T05:00:04Z", "output": m2_out}
    m2_ev["call"] = {"request": {"contents": []}, "response": {"candidates": []}}
    t_ev = {"event": "tool", "id": "tc-1", "timestamp": "2026-10-14T05:00:02Z"}
    t_ev.update({"completed": "2026-10-14T05:00:03Z", "arguments": {"cmd": "pytest -q"}})
    a_parts = [{"type": "reasoning", "reasoning": "Think."}, {"type": "text", "text": "Act."}]
    a_tc = [{"id": "tc-1", "function": "bash", "arguments": {"cmd": "pytest -q"}}]
    a_msg = {"id": "msg-1", "role": "assistant", "content": a_parts, "tool_calls": a_tc}
    t_msg = {"role": "tool", "tool_call_id": "tc-1", "content": "attachment://att_t", "error": "w"}
    ev_data = {"messages": [{"role": "system", "content": "attachment://att_s"}]}
    ev_data["calls"] = [{"role": "user", "parts": [{"text": "Run pytest -q"}]}]
    msgs = [{"role": "system", "content": "attachment://att_s"}, {"role": "user", "content": "Run"}]
    msgs.extend([a_msg, t_msg, {"id": "msg-2", "role": "assistant", "content": "Done."}])
    sample = {"id": "s-1", "epoch": 1, "scores": {"acc": {"value": 1.0}}, "events_data": ev_data}
    sample["attachments"] = {"att_s": "System prompt", "att_t": "1 passed"}
    sample["messages"] = msgs
    sample["events"] = [m1_ev, m_err, m2_ev, t_ev, {"event": "sandbox", "cmd": "pytest -q"}]
    eval_meta = {"run_id": "run-123", "task": "vimgolf", "model": "google/gemini-3.5-flash"}
    _stub_eval_logs(monkeypatch, {eval_file: {"eval": eval_meta, "samples": [sample]}})

    (traces_dir / "vm-abc.warc.gz").write_bytes(b"warc")
    (traces_dir / "vm-abc.meta.json").write_text(json.dumps({"vm_id": "vm-abc"}), encoding="utf-8")
    row = {"id": 1, "timestamp": "2026-10-14T05:00:02.5Z", "exec_id": 1, "command": "pytest -q"}
    _write_rows_ndjson_gz(traces_dir / "vm-abc.rows.ndjson.gz", "vm-abc", {"exec_events": [row]})

    monkeypatch.setattr("inspect_capsem.trace_export._has_harbor_validator", lambda: False)
    assert main([str(tmp_path), "--out-dir", str(traces_dir), "--validate", "--json"]) == 0
    summary = export_traces([eval_file], out_dir=traces_dir, validate=True)
    assert summary["validation_errors"] == [] and summary["steps_with_llm_call"] == 2
    assert summary["trajectory_coverage"][0]["errored_model_events_skipped"] == 1
    traj = json.loads(Path(summary["trajectory_files"][0]).read_text(encoding="utf-8"))
    assert traj["steps"][2]["metrics"]["prompt_tokens"] == 42
    assert traj["steps"][3]["metrics"]["prompt_tokens"] == 50
    assert "exec_events" in traj["steps"][2]["observation"]["results"][0]["extra"]["capsem_events"]
    assert traj["extra"]["capsem_session_rows"] == "vm-abc.rows.ndjson.gz"
    assert traj["extra"]["capsem_session_warc"] == "vm-abc.warc.gz"


def test_convert_eval_log_and_signal_forwarding(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    eval_nt = tmp_path / "notool.eval"
    msgs = [{"role": "user", "content": "hi"}, {"role": "assistant", "content": "ok"}]
    evs = [{"event": "sandbox", "timestamp": "2026-10-14T05:00:02Z", "cmd": "ls"}]
    sample = {"id": "s1", "messages": msgs, "events": evs}
    _stub_eval_logs(monkeypatch, {eval_nt: {"eval": {"run_id": "r2"}, "samples": [sample]}})
    row = {"id": 1, "timestamp": "2026-10-14T05:00:02Z", "exec_id": 1, "command": "ls"}
    _write_rows_ndjson_gz(tmp_path / "vm-2.rows.ndjson.gz", "vm-2", {"exec_events": [row]})
    monkeypatch.setattr("inspect_capsem.trace_export._has_harbor_validator", lambda: False)
    written = convert_eval_log(eval_nt, tmp_path / "out_nt", trace_db_dir=tmp_path)
    traj_nt = json.loads(written[0].read_text(encoding="utf-8"))
    assert "exec_events" in traj_nt["steps"][1]["extra"]["capsem_events"]

    empty_res = {"total_samples": 0, "eval_files": [], "steps_with_llm_call": 0}
    empty_res.update({"total_agent_steps": 0, "steps_without_llm_call": 0})
    empty_res.update({"samples_with_capsem_db": 0, "validation_errors": []})
    py_check = "import os; assert os.environ['INSPECT_EVAL_LOG_MODEL_API'] == '1'"
    cmd = [sys.executable, "-c", py_check]
    monkeypatch.setattr("inspect_capsem.trace_export.export_traces", lambda *_a, **_kw: empty_res)
    sig_dir = tmp_path / "s"
    assert run_with_signal_forwarding(cmd, log_dir=tmp_path, trace_dir=sig_dir, validate=True) == 0
    monkeypatch.setattr(
        "inspect_capsem.trace_export.export_traces",
        lambda *_a, **_kw: (_ for _ in ()).throw(RuntimeError("boom")),
    )
    assert run_with_signal_forwarding(cmd, log_dir=tmp_path, trace_dir=sig_dir, validate=True) == 1
    assert "Failed to export traces" in caplog.text
