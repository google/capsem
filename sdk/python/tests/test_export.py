"""Hermetic unit tests for `vm.export` (`quiesce`, `rows`, `warc`, `events`, `atif`) & `WarcBodyReader`."""

from __future__ import annotations

import asyncio
import json
from pathlib import Path
from types import SimpleNamespace
from typing import Any

from capsem import VM, HttpError, Hypervisor
from capsem.trace import (
    EXPORT_PERSIST_PREFIX, WarcBodyReader, build_steps_from_session_tables,
    cleanup_export_persisted_vm, enrich_steps_by_turn_id, is_export_persisted_vm,
    quiesce_vm_for_export, rows_to_atif, validate_atif_trajectory,
)

from .facade_gateway import gateway
from .test_trace import populate_session_exports


def test_vm_export_rows_warc_events_and_atif_modes(tmp_path: Path) -> None:
    rows_path, warc_path = populate_session_exports(
        tmp_path / "fixture", "vm-0", with_models=True, with_exec=True, include_security=True
    )
    sys_txt = "Full system prompt from archive"

    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            state.rows_bytes = rows_path.read_bytes()
            state.warc_bytes = warc_path.read_bytes()
            vm = hv.vm(id="vm-0")
            assert await vm.export.rows() == state.rows_bytes
            rows_file = tmp_path / "out" / "rows.ndjson.gz"
            assert await vm.export.rows(rows_file, tables=["model_calls", "exec_events"]) == rows_file
            assert rows_file.read_bytes() == state.rows_bytes
            assert await vm.export.warc() == state.warc_bytes
            warc_file = tmp_path / "out" / "bodies.warc.gz"
            assert await vm.export.warc(warc_file) == warc_file
            assert warc_file.read_bytes() == state.warc_bytes

            events_dir = tmp_path / "events"
            ev_rows, ev_warc = await vm.export.events(
                events_dir, include_security_events=False, labels={"env": "test"}, write_meta=True
            )
            assert ev_rows.is_file() and ev_warc.is_file()
            assert ev_rows.name == "vm-0.rows.ndjson.gz" and ev_warc.name == "vm-0.warc.gz"
            meta = json.loads((events_dir / "vm-0.meta.json").read_text())
            assert meta["rows_ndjson_gz"] == "vm-0.rows.ndjson.gz"
            assert meta["warc_gz"] == "vm-0.warc.gz" and meta["labels"] == {"env": "test"}

            atif_http = await vm.export.atif(output_path=tmp_path / "atif_http.json")
            assert atif_http["extra"]["capsem_summary"]["bodies"] == "http"
            assert atif_http["steps"][0]["source"] == "system"
            assert atif_http["steps"][0]["message"] == sys_txt
            assert atif_http["steps"][1]["source"] == "user"
            assert atif_http["steps"][1]["message"] == "hi"
            step1 = atif_http["steps"][2]
            assert step1["observation"]["results"][0]["content"] == "On branch main (full hydrated tool output)"
            assert "llm_call" not in step1["extra"]

            atif_warc = await vm.export.atif(bodies="warc")
            assert atif_warc["extra"]["capsem_summary"]["bodies"] == "warc"
            assert atif_warc["steps"][0]["message"] == sys_txt
            assert atif_warc["steps"][2]["observation"]["results"][0]["content"] == (
                "On branch main (full hydrated tool output)"
            )

            atif_full = await vm.export.atif(bodies="full", include_security_events=True)
            s1_full = atif_full["steps"][2]
            assert s1_full["extra"]["llm_call"]["request"]["system"] == sys_txt
            s1_events = s1_full["extra"]["capsem_events"]
            assert s1_events["exec_events"][0]["stdout"] == "On branch main (full stdout)"
            assert s1_events["security_rule_events"][0]["payload"] == {"event_type": "process.exec", "rule": "r1"}

            def _st(sid: int, ts: str, src: str, msg: str) -> dict[str, Any]:
                return {"step_id": sid, "timestamp": ts, "source": src, "message": msg}

            ext_traj: dict[str, Any] = {
                "schema_version": "ATIF-v1.7", "session_id": "ext-1",
                "agent": {"name": "ext", "version": "1.0"},
                "steps": [
                    _st(1, "2026-10-14T08:00:00Z", "user", "go"),
                    _st(2, "2026-10-14T08:00:01Z", "agent", "s1"),
                    _st(3, "2026-10-14T08:00:05Z", "agent", "s2"),
                ],
            }
            ext_path = tmp_path / "ext_in.json"
            ext_path.write_text(json.dumps(ext_traj), encoding="utf-8")
            enriched = await vm.export.atif(ext_path, bodies="previews")
            assert enriched["steps"][1]["extra"]["capsem_model_call"]["turn_id"] == "turn-1"
            enriched_dict = await vm.export.atif(ext_traj, bodies="none")
            assert enriched_dict["extra"]["capsem_summary"]["bodies"] == "none"

    asyncio.run(run())


def test_vm_export_degradation_and_quiesce(tmp_path: Path) -> None:
    rows_path, _ = populate_session_exports(tmp_path / "fixture", "vm-0", with_models=True)
    assert is_export_persisted_vm(
        SimpleNamespace(persistent=True, name=f"{EXPORT_PERSIST_PREFIX}abc")
    )
    assert not is_export_persisted_vm(SimpleNamespace(persistent=True, name="my-dev-vm"))
    assert not is_export_persisted_vm(
        SimpleNamespace(persistent=False, name=f"{EXPORT_PERSIST_PREFIX}abc")
    )

    async def run() -> None:
        async with gateway() as (url, state):
            state.rows_bytes = rows_path.read_bytes()
            state.warc_bytes = b""
            async with VM(url, "token", id="vm-0") as vm:
                reader = WarcBodyReader(b"")
                assert reader.get_event_bodies("000000000001") == ()
                degraded = await vm.export.atif(bodies="http", quiesce=True, cleanup_persisted=True)
                assert degraded["steps"][0]["message"] == "You are helpful."
                await vm.export.events(tmp_path / "quiesced", quiesce=True, cleanup_persisted=True)
                assert not await vm.export.cleanup_persisted()
                disarmed: list[bool] = []
                mgd_vm = VM._bind(
                    vm._transport, id="vm-0", managed=True, on_stop=lambda: disarmed.append(True),
                )
                req_before = len(state.requests)
                assert await mgd_vm.export.quiesce()
                assert disarmed == [True]
                mgd_paths = [p for _, p, _ in state.requests[req_before:]]
                assert "/vms/vm-0/stop" in mgd_paths and "/vms/vm-0/persist" not in mgd_paths

            events: list[str] = []

            async def _ok_persist(name: str) -> None:
                events.append(f"persist:{name}")

            async def _conflict_persist(_name: str) -> None:
                raise HttpError(400, '{"error":"VM \\"vm-p\\" is already persistent"}')

            async def _managed_persist(_name: str) -> None:
                raise HttpError(409, '{"error":"managed sessions cannot become persistent"}')

            async def _name_conflict(_name: str) -> None:
                raise HttpError(400, '{"error":"persistent VM name already taken"}')

            tries = 0

            async def _retry_ok_persist(name: str) -> None:
                nonlocal tries
                tries += 1
                if tries == 1:
                    raise HttpError(400, '{"error":"persistent VM name already exists"}')
                events.append(f"persist:{name[:11]}")

            async def _rec_stop() -> None:
                events.append("stop")

            async def _rec_del() -> None:
                events.append("delete")

            async def _fail_stop() -> None:
                raise RuntimeError("stop failed")

            async def _info_stopped() -> SimpleNamespace:
                return SimpleNamespace(status="stopped", persistent=True, name="user-vm")

            async def _info_pr() -> SimpleNamespace:
                return SimpleNamespace(status="running", persistent=True, name="export-vm-pr")

            async def _info_404() -> SimpleNamespace:
                raise HttpError(404, '{"error":"not found"}')

            assert not await quiesce_vm_for_export(None, "vm-0")
            assert not await quiesce_vm_for_export(SimpleNamespace(), "vm-0")
            vm_s = SimpleNamespace(info=_info_stopped, persist=_ok_persist, stop=_rec_stop, delete=_rec_del)
            assert await quiesce_vm_for_export(vm_s, "vm-s") and events == []
            assert not await cleanup_export_persisted_vm(vm_s)
            vm_pr = SimpleNamespace(info=_info_pr, persist=_ok_persist, stop=_rec_stop, delete=_rec_del)
            assert await quiesce_vm_for_export(vm_pr, "vm-pr") and events == ["stop"]
            assert await cleanup_export_persisted_vm(vm_pr) and events[-1] == "delete"
            vm_404 = SimpleNamespace(info=_info_404, persist=_ok_persist, stop=_rec_stop)
            assert not await quiesce_vm_for_export(vm_404, "vm-404")
            full_id = "20261017-120000-0001"
            vm_ok = SimpleNamespace(persist=_ok_persist, stop=_rec_stop, delete=_rec_del)
            vm_conf = SimpleNamespace(persist=_conflict_persist, stop=_rec_stop)
            vm_mgd = SimpleNamespace(_managed=True, persist=_managed_persist, stop=_rec_stop)
            vm_dup = SimpleNamespace(persist=_name_conflict, stop=_rec_stop)
            vm_retry = SimpleNamespace(persist=_retry_ok_persist, stop=_rec_stop)
            vm_fail = SimpleNamespace(persist=_ok_persist, stop=_fail_stop)
            assert await quiesce_vm_for_export(vm_ok, full_id)
            assert await cleanup_export_persisted_vm(vm_ok) and not vm_ok._export_persisted
            assert await quiesce_vm_for_export(vm_conf, "vm-p")
            assert await quiesce_vm_for_export(vm_mgd, "vm-m")
            assert not await quiesce_vm_for_export(vm_dup, "vm-d")
            assert await quiesce_vm_for_export(vm_retry, "vm-r")
            assert not await quiesce_vm_for_export(vm_fail, "vm-f")
            assert not await cleanup_export_persisted_vm(None)
            assert events == [
                "stop", "delete", f"persist:export-{full_id}", "stop", "delete",
                "stop", "stop", "persist:export-vm-r", "stop", "persist:export-vm-f",
            ]

    asyncio.run(run())


def test_gemini_cli_tool_response_pairing_prompts_and_ambient_turn_id() -> None:
    trunc_req = (
        '{"model":"models/gemini-2.5-flash","contents":[{"role":"user","parts":'
        '[{"text":"Please fix the git repository"}]}],"systemInstruction":{"parts":[{"text":"trunc'
    )
    model_calls = [
        {
            "id": 1, "timestamp": "2026-10-14T18:52:48Z", "provider": "google",
            "model": "gemini-2.5-flash", "system_prompt_preview": "You are Gemini CLI.",
            "request_body_preview": trunc_req, "turn_id": "turn-uuid-1",
            "input_tokens": 100, "output_tokens": 30,
        },
        {
            "id": 2, "timestamp": "2026-10-14T18:52:53Z", "provider": "google",
            "model": "gemini-2.5-flash", "turn_id": "turn-uuid-2",
            "input_tokens": 150, "output_tokens": 25,
        },
        {
            "id": 3, "timestamp": "2026-10-14T18:52:59Z", "provider": "google",
            "model": "gemini-2.5-flash", "text_content": "Repository fixed.",
            "turn_id": "vm-ambient-tid", "input_tokens": 180, "output_tokens": 15,
        },
    ]

    def _tc(mc: int, cid: str, fn: str, args: str) -> dict[str, Any]:
        return {"model_call_id": mc, "call_id": cid, "tool_name": fn, "arguments": args}

    def _tr(mc: int, cid: str, prev: str) -> dict[str, Any]:
        return {"model_call_id": mc, "call_id": cid, "content_preview": prev, "is_error": 0}

    tool_calls = [
        _tc(1, "gemini_update_topic_0", "update_topic", '{"topic":"Inspect"}'),
        _tc(1, "gemini_run_shell_command_1", "run_shell_command", '{"command":"git status"}'),
        _tc(2, "gemini_update_topic_0", "update_topic", '{"topic":"Merge"}'),
        _tc(2, "gemini_run_shell_command_1", "run_shell_command", '{"command":"git merge fix"}'),
    ]
    tool_responses = [
        _tr(2, "update_topic_1792003967212_0", "ok-topic-1"),
        _tr(2, "run_shell_command_1792003967253_1", "On branch master"),
        _tr(3, "update_topic_1792003970888_0", "ok-topic-2"),
        _tr(3, "run_shell_command_1792003970942_1", "Merged fix"),
    ]
    steps = build_steps_from_session_tables(model_calls, [], tool_calls, tool_responses)
    assert [s["source"] for s in steps] == ["system", "user", "agent", "agent", "agent"]
    assert steps[0]["message"] == "You are Gemini CLI."
    assert steps[1]["message"] == "Please fix the git repository"
    s2_res, s3_res = steps[2]["observation"]["results"], steps[3]["observation"]["results"]
    assert [r["content"] for r in s2_res] == ["ok-topic-1", "On branch master"]
    assert [r["source_call_id"] for r in s2_res] == [
        "gemini_update_topic_0", "gemini_run_shell_command_1"
    ]
    assert [r["content"] for r in s3_res] == ["ok-topic-2", "Merged fix"]
    amb, g_dom = "vm-ambient-tid", "aiplatform.googleapis.com"
    events = {
        "exec_events": [
            {"timestamp": "2026-10-14T18:52:05Z", "command": "setup", "turn_id": amb},
            {"timestamp": "2026-10-14T18:52:49Z", "command": "git status", "turn_id": amb},
            {"timestamp": "2026-10-14T18:52:55Z", "command": "git merge fix", "turn_id": amb},
        ],
        "net_events": [
            {"timestamp": "2026-10-14T18:52:10Z", "domain": "registry.npmjs.org", "turn_id": amb},
            {"timestamp": "2026-10-14T18:52:48Z", "domain": g_dom, "turn_id": "turn-uuid-1"},
            {"timestamp": "2026-10-14T18:52:53Z", "domain": g_dom, "turn_id": "turn-uuid-2"},
            {"timestamp": "2026-10-14T18:52:59Z", "domain": g_dom, "turn_id": amb},
        ],
    }
    enrich_steps_by_turn_id(steps, model_calls, events)
    s2_ex = steps[2]["extra"]["capsem_events"]["exec_events"]
    s3_ex = steps[3]["extra"]["capsem_events"]["exec_events"]
    assert [e["command"] for e in s2_ex] == ["git status"]
    assert [e["command"] for e in s3_ex] == ["git merge fix"]
    assert steps[4]["extra"]["capsem_events"]["exec_events"] == []
    assert len(steps[4]["extra"]["capsem_events"]["net_events"]) == 1
    traj = {
        "schema_version": "ATIF-v1.7", "session_id": "s",
        "agent": {"name": "g", "version": "1"}, "steps": steps,
    }
    assert validate_atif_trajectory(traj) == []


def test_recorded_gemini_cli_trial_fixture_to_atif() -> None:
    fixtures_dir = Path(__file__).parent / "fixtures"
    rows_p = fixtures_dir / "gemini_cli_trial.rows.ndjson.gz"
    warc_p = fixtures_dir / "gemini_cli_trial.warc.gz"
    traj = rows_to_atif(rows_p, warc_path=warc_p, bodies="warc")
    assert validate_atif_trajectory(traj) == []
    assert len(traj["steps"]) == 12
    summary = traj["extra"]["capsem_summary"]
    assert summary["model_calls_count"] == 10
    assert summary["exec_events_count"] == 34
    assert summary["fs_events_count"] == 27
    assert summary["net_events_count"] == 80
    assert summary["bodies"] == "warc"
    full_traj = rows_to_atif(rows_p, warc_path=warc_p, bodies="full")
    assert "llm_call" in full_traj["steps"][2]["extra"]
