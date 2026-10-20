"""Join Capsem ledger event tables onto ATIF-v1.7 steps and validate trajectories."""

from __future__ import annotations

import datetime as dt
import json
import re
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Any

from ._trace_ledger import HARBOR_EVENT_TABLES, parse_iso_datetime

_MC_SUMMARY_KEYS = (
    "id", "event_id", "timestamp", "provider", "protocol", "model", "process_name",
    "method", "path", "system_prompt_preview", "messages_count", "tools_count",
    "stop_reason", "input_tokens", "output_tokens", "duration_ms", "ttfb_ms",
    "estimated_cost_usd", "trace_id", "turn_id",
)
_AMBIENT_TABLES = ("audit_events", "exec_events", "fs_events")


def _row_datetime(row: Mapping[str, Any]) -> dt.datetime | None:
    if (d := parse_iso_datetime(row.get("timestamp") or row.get("created_at"))) is not None:
        return d
    if (unix_ms := row.get("timestamp_unix_ms")) is not None:
        try:
            return dt.datetime.fromtimestamp(float(unix_ms) / 1000.0, tz=dt.UTC)
        except (TypeError, ValueError, OSError):
            return None
    return None


def _mc_start_dt(mc: Mapping[str, Any]) -> dt.datetime | None:
    end_dt = _row_datetime(mc)
    if end_dt is None:
        return None
    dur_ms = int(mc.get("duration_ms") or 0)
    return end_dt - dt.timedelta(milliseconds=max(dur_ms, 0)) if dur_ms > 0 else end_dt


def _mc_summary(mc: Mapping[str, Any]) -> dict[str, Any]:
    usage_raw = mc.get("usage_details")
    usage: Any = usage_raw
    if isinstance(usage_raw, str) and usage_raw.strip():
        try:
            usage = json.loads(usage_raw)
        except ValueError:
            usage = usage_raw
    return {k: mc.get(k) for k in _MC_SUMMARY_KEYS} | {"usage_details": usage}


def _in_window(
    rdt: dt.datetime | None, start: dt.datetime | None, end: dt.datetime | None
) -> bool:
    return bool(rdt is not None and (start is None or rdt >= start) and (end is None or rdt < end))


def _step_extra(step: dict[str, Any]) -> dict[str, Any]:
    if not isinstance(step.get("extra"), dict):
        step["extra"] = {}
    return step["extra"]


def enrich_steps_by_turn_id(
    steps: list[dict[str, Any]],
    model_calls: Sequence[Mapping[str, Any]],
    events: Mapping[str, Sequence[dict[str, Any]]],
    *,
    event_tables: Sequence[str] | None = None,
) -> bool:
    """Attach `model_calls` and `events` to agent steps by `turn_id` and time windows."""
    agent_steps = [s for s in steps if s.get("source") == "agent"]
    if not agent_steps:
        return False
    eff_tables = tuple(event_tables) if event_tables is not None else tuple(events.keys())
    calls_by_turn: dict[str, list[Mapping[str, Any]]] = {}
    turn_order: list[str] = []
    for mc in model_calls:
        tid = str(mc.get("turn_id") or "").strip()
        if tid and tid not in calls_by_turn:
            calls_by_turn[tid] = []
            turn_order.append(tid)
        if tid:
            calls_by_turn[tid].append(mc)

    call_dts = [_mc_start_dt(mc) for mc in model_calls]
    call_end_dts = [_row_datetime(mc) for mc in model_calls]
    is_synth = len(agent_steps) == len(model_calls) and all(
        isinstance(s.get("extra"), Mapping) and s["extra"].get("capsem_model_call_id") is not None
        for s in agent_steps
    )
    if len(turn_order) <= 1 and len(model_calls) > 1 and len(agent_steps) > 1:
        j_mode = "timestamp_window" if is_synth else "ordinal_fallback"
        dated_by_tbl = {
            tbl: [(_row_datetime(r), r) for r in events.get(tbl, ())] for tbl in eff_tables
        }
        for idx, step in enumerate(agent_steps[: len(model_calls)]):
            mc, win_s = model_calls[idx], call_dts[idx]
            win_e = call_dts[idx + 1] if idx + 1 < len(call_dts) else None
            matched: dict[str, Any] = {
                "join_mode": j_mode,
                "call_index": idx,
                "turn_id": mc.get("turn_id"),
            }
            for tbl in eff_tables:
                rows = dated_by_tbl[tbl]
                matched[tbl] = (
                    [r for rdt, r in rows if _in_window(rdt, win_s, win_e)]
                    if win_s
                    else [r for _rdt, r in rows]
                )
            _step_extra(step).update(capsem_model_call=_mc_summary(mc), capsem_events=matched)
        return True

    turn_spans: dict[str, tuple[dt.datetime | None, dt.datetime | None]] = {}
    for t in turn_order:
        idxs = [i for i, m in enumerate(model_calls) if str(m.get("turn_id") or "").strip() == t]
        if idxs:
            max_i = max(idxs)
            turn_spans[t] = (
                call_dts[min(idxs)],
                call_dts[max_i + 1] if max_i + 1 < len(call_dts) else None,
            )

    prepared_tables: dict[
        str,
        tuple[
            bool, bool, dict[str, list[dict[str, Any]]],
            list[tuple[dt.datetime, dict[str, Any]]], list[tuple[dt.datetime, dict[str, Any]]],
        ],
    ] = {}
    for tbl in eff_tables:
        raw_rows = events.get(tbl, ())
        dated_rows = [(_row_datetime(r), str(r.get("turn_id") or "").strip(), r) for r in raw_rows]
        ambient_tids = {
            rt
            for rdt, rt, _r in dated_rows
            if len(turn_order) > 1
            and rt in turn_spans
            and rdt is not None
            and not _in_window(rdt, *turn_spans[rt])
        }
        valid_tids = set(calls_by_turn) - ambient_tids
        has_valid = any(rt in valid_tids for _rdt, rt, _r in dated_rows)
        by_tid: dict[str, list[dict[str, Any]]] = {}
        unmatched_dated: list[tuple[dt.datetime, dict[str, Any]]] = []
        all_dated: list[tuple[dt.datetime, dict[str, Any]]] = []
        for rdt, rt, r in dated_rows:
            if rdt is not None:
                all_dated.append((rdt, r))
            if rt in valid_tids:
                by_tid.setdefault(rt, []).append(r)
            elif rdt is not None:
                unmatched_dated.append((rdt, r))
        prepared_tables[tbl] = (has_valid, bool(ambient_tids), by_tid, unmatched_dated, all_dated)

    for idx, step in enumerate(agent_steps):
        extra = _step_extra(step)
        ex_tid = extra.get("turn_id")
        fb_tid = (
            str(model_calls[idx]["turn_id"]).strip()
            if idx < len(model_calls) and model_calls[idx].get("turn_id")
            else None
        )
        tid = (
            ex_tid
            if isinstance(ex_tid, str) and ex_tid in calls_by_turn
            else (turn_order[idx] if idx < len(turn_order) else fb_tid)
        )
        if not tid:
            continue
        t_calls = calls_by_turn.get(tid, [])
        if t_calls:
            extra["capsem_model_call"] = (
                _mc_summary(t_calls[0]) if len(t_calls) == 1 else [_mc_summary(c) for c in t_calls]
            )
        step_events: dict[str, Any] = {"join_mode": "turn_id", "turn_id": tid}
        s_dt = call_dts[idx] if idx < len(call_dts) else None
        next_dt = call_dts[idx + 1] if idx + 1 < len(call_dts) else None
        end_cap = (
            (end_dt + dt.timedelta(milliseconds=50))
            if idx < len(call_end_dts) and (end_dt := call_end_dts[idx]) is not None
            else None
        )
        has_tools = bool(step.get("tool_calls"))
        for tbl, (has_valid, has_ambient, by_tid, unmatched, all_dated) in prepared_tables.items():
            e_dt = (
                next_dt
                if next_dt is not None
                else (end_cap if has_ambient and not has_tools else None)
            )
            if has_valid:
                tbl_matches = list(by_tid.get(tid, ()))
                if s_dt is not None:
                    tbl_matches.extend(r for rdt, r in unmatched if _in_window(rdt, s_dt, e_dt))
                step_events[tbl] = tbl_matches
            elif tbl == "substitution_events":
                step_events[tbl] = list(events.get(tbl, ())) if idx == 0 else []
            elif has_ambient and not has_tools and tbl in _AMBIENT_TABLES:
                step_events[tbl] = []
            else:
                step_events[tbl] = (
                    [r for rdt, r in all_dated if _in_window(rdt, s_dt, e_dt)] if s_dt else []
                )
        extra["capsem_events"] = step_events
    return True


def enrich_steps_by_timestamp_window(
    steps: list[dict[str, Any]],
    events: Mapping[str, Sequence[dict[str, Any]]],
    *,
    event_tables: Sequence[str] | None = None,
) -> None:
    """Attach `events` to steps using `[step_ts, next_step_ts)` timestamp windows."""
    eff_tables = tuple(event_tables) if event_tables is not None else tuple(events.keys())
    step_dts = [parse_iso_datetime(s.get("timestamp")) for s in steps]
    dated_tables: dict[str, list[tuple[dt.datetime, dict[str, Any]]]] = {
        t: [(rdt, r) for r in events.get(t, ()) if (rdt := _row_datetime(r)) is not None]
        for t in eff_tables
    }
    for idx, step in enumerate(steps):
        start_dt = step_dts[idx]
        end_dt = next((step_dts[j] for j in range(idx + 1, len(steps)) if step_dts[j]), None)
        matched = {
            t: [r for rdt, r in dated_tables[t] if _in_window(rdt, start_dt, end_dt)]
            if start_dt is not None
            else (list(events.get(t, ())) if idx == len(steps) - 1 else [])
            for t in eff_tables
        }
        if step.get("source") == "agent" or any(matched.values()):
            w_end = steps[idx + 1].get("timestamp") if idx + 1 < len(steps) else None
            _step_extra(step)["capsem_events"] = {
                "join_mode": "timestamp_window",
                "window_start": step.get("timestamp"),
                "window_end": w_end,
                **matched,
            }


def attach_capsem_summary(
    trajectory_data: dict[str, Any],
    *,
    vm_id: str,
    rows_path: Path | None = None,
    warc_path: Path | None = None,
    model_calls_count: int,
    events: Mapping[str, Sequence[dict[str, Any]]],
    dropped_security_tables: bool,
    join_mode: str,
    bodies: str = "warc",
) -> None:
    """Attach `extra.capsem_summary` metadata onto `trajectory_data`."""
    tbl_keys = (*HARBOR_EVENT_TABLES, *(t for t in events if t not in HARBOR_EVENT_TABLES))
    counts = {f"{t}_count": len(events.get(t, ())) for t in tbl_keys}
    _step_extra(trajectory_data)["capsem_summary"] = {
        "vm_id": vm_id,
        "rows_ndjson_gz": rows_path.name if rows_path is not None else None,
        "warc_gz": warc_path.name if warc_path is not None else None,
        "bodies": bodies,
        "join_mode": join_mode,
        "dropped_security_tables": dropped_security_tables,
        "model_calls_count": model_calls_count,
        **counts,
    }


def validate_atif_trajectory(
    trajectory_data: Mapping[str, Any], *, label: str = "<trajectory>"
) -> list[str]:
    """Validate structural ATIF-v1.* invariants using stdlib only."""
    errs: list[str] = []
    ver = trajectory_data.get("schema_version")
    if not (isinstance(ver, str) and re.match(r"^ATIF-v1\.\d+$", ver)):
        errs.append(f"{label}: expected schema_version 'ATIF-v1.*', got {ver!r}")
    if not str(trajectory_data.get("session_id") or "").strip():
        errs.append(f"{label}: missing non-empty session_id")
    agent = trajectory_data.get("agent")
    if not isinstance(agent, Mapping) or not str(agent.get("name") or "").strip():
        errs.append(f"{label}: missing agent.name")
    steps = trajectory_data.get("steps")
    if not isinstance(steps, Sequence) or not steps:
        return [*errs, f"{label}: steps must be a non-empty list"]
    for i, step in enumerate(steps, start=1):
        if not isinstance(step, Mapping):
            errs.append(f"{label}: step[{i}] must be a mapping")
            continue
        if step.get("step_id") != i:
            errs.append(f"{label}: step[{i}] has non-sequential step_id={step.get('step_id')!r}")
        if step.get("source") not in ("system", "user", "agent"):
            errs.append(f"{label}: step[{i}] invalid source={step.get('source')!r}")
        if not isinstance(step.get("message"), (str, list)):
            errs.append(f"{label}: step[{i}] missing string or list 'message'")
        extra = step.get("extra")
        llm_call = extra.get("llm_call") if isinstance(extra, Mapping) else None
        if llm_call is not None and "attachment://" in json.dumps(llm_call):
            errs.append(f"{label}: step[{i}] has unresolved attachment:// pointer")
    return errs
