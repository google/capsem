"""ATIF-v1.7 trajectory synthesis and enrichment over Capsem `rows.ndjson.gz` + `export.warc.gz`."""

from __future__ import annotations

import copy
import json
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Any

import capsem

from ._trace_atif import (
    attach_capsem_summary,
    build_steps_from_session_tables,
    enrich_steps_by_timestamp_window,
    enrich_steps_by_turn_id,
    validate_atif_trajectory,
)
from ._trace_bodies import (
    BodiesMode,
    BodyReader,
    WarcBodyReader,
    candidate_body_event_ids,
    decode_security_payload,
    hydrate_tables_from_bodies,
    parse_warc_records,
    validate_event_id,
)
from ._trace_ledger import (
    ALL_EXPORT_TABLES,
    DEFAULT_EXPORT_TABLES,
    DROPPED_SECURITY_TABLES,
    EXPORT_PERSIST_PREFIX,
    HARBOR_EVENT_TABLES,
    ROWS_STREAM_FORMAT,
    atomic_write_bytes,
    cleanup_export_persisted_vm,
    has_session_events,
    is_export_persisted_vm,
    load_rows_tables,
    parse_iso_datetime,
    parse_rows_ndjson_gz,
    quiesce_vm_for_export,
    resolve_include_security_events,
    resolve_warc_path,
    write_export_meta,
)

_VALID_BODIES_MODES: tuple[str, ...] = ("none", "previews", "warc", "http", "full")
_STEP_TABLES = ("model_calls", "model_items", "tool_calls", "tool_responses", "event_body_blobs")


def _validate_bodies_mode(bodies: str) -> None:
    if bodies not in _VALID_BODIES_MODES:
        raise ValueError(f"Invalid bodies mode {bodies!r}; expected one of {_VALID_BODIES_MODES}")


def _effective_event_tables(
    event_tables: Sequence[str] | None, *, include_security: bool
) -> tuple[str, ...]:
    base = tuple(event_tables) if event_tables is not None else HARBOR_EVENT_TABLES
    if include_security:
        return base + tuple(t for t in DROPPED_SECURITY_TABLES if t not in base)
    return base


def _resolve_vm_id(
    rows_source: str | Path | bytes | Mapping[str, Any], vm_id: str | None
) -> tuple[str, Path | None]:
    rows_path = Path(rows_source) if isinstance(rows_source, (str, Path)) else None
    if vm_id:
        return vm_id, rows_path
    if rows_path is not None:
        name = rows_path.name
        for suffix in (".rows.ndjson.gz", ".events.ndjson.gz", ".ndjson.gz", ".ndjson"):
            if name.endswith(suffix):
                return name.removesuffix(suffix) or rows_path.stem, rows_path
        return rows_path.stem, rows_path
    return "capsem-vm", None


def _resolve_reader(
    bodies: str,
    body_reader: BodyReader | None,
    rows_path: Path | None,
    warc_source: str | Path | bytes | None,
) -> tuple[BodyReader | None, Path | None]:
    warc_p: Path | None = None
    if isinstance(warc_source, (str, Path)):
        cand = Path(warc_source)
        warc_p = cand if cand.is_file() else None
    elif warc_source is None and rows_path is not None:
        warc_p = resolve_warc_path(rows_path)
    if bodies not in ("warc", "http", "full"):
        return None, warc_p
    if body_reader is not None:
        return body_reader, warc_p
    if warc_source is not None:
        return WarcBodyReader(warc_source), warc_p
    if warc_p is not None:
        return WarcBodyReader(warc_p), warc_p
    return None, None


def _exec_only_steps(exec_rows: Sequence[Mapping[str, Any]]) -> list[dict[str, Any]]:
    first_ts = exec_rows[0].get("timestamp") if exec_rows else None
    msg = "(Capsem exec-only session)"
    steps: list[dict[str, Any]] = [
        {"step_id": 1, "timestamp": first_ts, "source": "user", "message": msg}
    ]
    for idx, ex in enumerate(exec_rows, start=2):
        cmd = str(ex.get("command") or "")
        out = str(ex.get("stdout") or ex.get("stdout_preview") or "")
        err = str(ex.get("stderr") or ex.get("stderr_preview") or "")
        call_id = f"exec_{ex.get('exec_id') or idx}"
        ex_meta = {"exit_code": ex.get("exit_code"), "duration_ms": ex.get("duration_ms")}
        steps.append({
            "step_id": idx,
            "timestamp": ex.get("timestamp"),
            "source": "agent",
            "message": cmd or f"exec #{idx - 1}",
            "llm_call_count": 0,
            "tool_calls": [
                {"tool_call_id": call_id, "function_name": "bash", "arguments": {"command": cmd}}
            ],
            "observation": {
                "results": [{
                    "source_call_id": call_id,
                    "content": out if not err else f"{out}\n{err}".strip(),
                    "extra": ex_meta,
                }]
            },
        })
    return steps


def rows_to_atif(
    rows_source: str | Path | bytes | Mapping[str, Sequence[Mapping[str, Any]]],
    *,
    vm_id: str | None = None,
    session_id: str | None = None,
    agent_name: str | None = None,
    agent_version: str | None = None,
    schema_version: str = "ATIF-v1.7",
    warc_path: str | Path | bytes | None = None,
    event_tables: Sequence[str] | None = None,
    include_security_events: bool | None = None,
    bodies: BodiesMode = "warc",
    body_reader: BodyReader | None = None,
    output_path: str | Path | None = None,
) -> dict[str, Any]:
    """Synthesize a complete ATIF-v1.7 trajectory from `rows.ndjson.gz` (+ optional `warc.gz`)."""
    _validate_bodies_mode(bodies)
    inc_sec = resolve_include_security_events(include_security_events)
    eff_vm_id, rp = _resolve_vm_id(rows_source, vm_id)
    eff_tables = _effective_event_tables(event_tables, include_security=inc_sec)
    loaded = load_rows_tables(rows_source, (*_STEP_TABLES, *eff_tables))
    reader, wp = _resolve_reader(bodies, body_reader, rp, warc_path)
    if reader is not None:
        hydrate_tables_from_bodies(loaded, reader, full=(bodies == "full"))
    mcalls, execs = loaded["model_calls"], loaded["exec_events"]
    if not mcalls and not execs:
        raise ValueError(
            f"Cannot synthesize ATIF from {eff_vm_id}: no model_calls or exec_events rows"
        )
    ev_map = {t: loaded.get(t, []) for t in eff_tables}
    if mcalls:
        steps = build_steps_from_session_tables(
            mcalls,
            loaded["model_items"],
            loaded["tool_calls"],
            loaded["tool_responses"],
            include_llm_call=(bodies == "full"),
        )
        enrich_steps_by_turn_id(steps, mcalls, ev_map, event_tables=eff_tables)
        j_mode = (steps[-1].get("extra") or {}).get("capsem_events", {}).get("join_mode", "turn_id")
    else:
        steps = _exec_only_steps(execs)
        enrich_steps_by_timestamp_window(steps, ev_map, event_tables=eff_tables)
        j_mode = "exec_only"
    first_mc = mcalls[0] if mcalls else {}
    in_tok = sum(int(m.get("input_tokens") or 0) for m in mcalls)
    out_tok = sum(int(m.get("output_tokens") or 0) for m in mcalls)
    cost = round(sum(float(m.get("estimated_cost_usd") or 0.0) for m in mcalls), 6)
    traj: dict[str, Any] = {
        "schema_version": schema_version,
        "session_id": session_id or eff_vm_id,
        "agent": {
            "name": agent_name or str(first_mc.get("process_name") or "capsem-agent"),
            "version": agent_version or capsem.__version__,
            **({"model_name": first_mc["model"]} if first_mc.get("model") else {}),
        },
        "steps": steps,
        "final_metrics": {
            "total_prompt_tokens": in_tok,
            "total_completion_tokens": out_tok,
            "total_steps": len(steps),
            **({"total_cost_usd": cost} if cost > 0 else {}),
        },
        "extra": {},
    }
    attach_capsem_summary(
        traj, vm_id=eff_vm_id, rows_path=rp, warc_path=wp, model_calls_count=len(mcalls),
        events=ev_map, dropped_security_tables=not inc_sec, join_mode=j_mode, bodies=bodies,
    )
    if errs := validate_atif_trajectory(traj, label=str(rp or eff_vm_id)):
        raise ValueError(f"Synthesized ATIF trajectory failed validation: {errs}")
    if output_path is not None:
        atomic_write_bytes(Path(output_path), (json.dumps(traj, indent=2) + "\n").encode("utf-8"))
    return traj


def enrich_atif_trajectory(
    trajectory: str | Path | Mapping[str, Any],
    rows_source: str | Path | bytes | Mapping[str, Sequence[Mapping[str, Any]]],
    *,
    vm_id: str | None = None,
    warc_path: str | Path | bytes | None = None,
    event_tables: Sequence[str] | None = None,
    include_security_events: bool | None = None,
    bodies: BodiesMode = "warc",
    body_reader: BodyReader | None = None,
    output_path: str | Path | None = None,
) -> dict[str, Any]:
    """Enrich an existing ATIF-v1.* trajectory dict or JSON file from `rows.ndjson.gz`."""
    _validate_bodies_mode(bodies)
    inc_sec = resolve_include_security_events(include_security_events)
    eff_vm_id, rp = _resolve_vm_id(rows_source, vm_id)
    traj: dict[str, Any] = (
        json.loads(Path(trajectory).read_text(encoding="utf-8"))
        if isinstance(trajectory, (str, Path))
        else copy.deepcopy(dict(trajectory))
    )
    eff_tables = _effective_event_tables(event_tables, include_security=inc_sec)
    loaded = load_rows_tables(rows_source, ("model_calls", "event_body_blobs", *eff_tables))
    reader, wp = _resolve_reader(bodies, body_reader, rp, warc_path)
    if reader is not None:
        hydrate_tables_from_bodies(loaded, reader, full=(bodies == "full"))
    mcalls = loaded["model_calls"]
    ev_map = {t: loaded.get(t, []) for t in eff_tables}
    steps: list[dict[str, Any]] = list(traj.get("steps") or [])
    if mcalls and enrich_steps_by_turn_id(steps, mcalls, ev_map, event_tables=eff_tables):
        agent_steps = [s for s in steps if s.get("source") == "agent"]
        j_mode = (
            (agent_steps[-1].get("extra") or {}).get("capsem_events", {}).get("join_mode")
            if agent_steps
            else "turn_id"
        ) or "turn_id"
    else:
        enrich_steps_by_timestamp_window(steps, ev_map, event_tables=eff_tables)
        j_mode = "timestamp_window"
    if not isinstance(traj.get("extra"), dict):
        traj["extra"] = {}
    attach_capsem_summary(
        traj, vm_id=eff_vm_id, rows_path=rp, warc_path=wp, model_calls_count=len(mcalls),
        events=ev_map, dropped_security_tables=not inc_sec, join_mode=j_mode, bodies=bodies,
    )
    if errs := validate_atif_trajectory(traj, label=str(rp or eff_vm_id)):
        raise ValueError(f"Enriched ATIF trajectory failed validation: {errs}")
    if output_path is not None:
        atomic_write_bytes(Path(output_path), (json.dumps(traj, indent=2) + "\n").encode("utf-8"))
    return traj


__all__ = [
    "ALL_EXPORT_TABLES", "DEFAULT_EXPORT_TABLES", "DROPPED_SECURITY_TABLES",
    "EXPORT_PERSIST_PREFIX", "HARBOR_EVENT_TABLES", "ROWS_STREAM_FORMAT",
    "BodiesMode", "BodyReader", "WarcBodyReader", "attach_capsem_summary",
    "build_steps_from_session_tables", "candidate_body_event_ids",
    "cleanup_export_persisted_vm", "decode_security_payload",
    "enrich_atif_trajectory", "enrich_steps_by_timestamp_window",
    "enrich_steps_by_turn_id", "has_session_events", "hydrate_tables_from_bodies",
    "is_export_persisted_vm", "load_rows_tables", "parse_iso_datetime",
    "parse_rows_ndjson_gz", "parse_warc_records", "quiesce_vm_for_export",
    "resolve_include_security_events", "resolve_warc_path", "rows_to_atif",
    "validate_atif_trajectory", "validate_event_id", "write_export_meta",
]
