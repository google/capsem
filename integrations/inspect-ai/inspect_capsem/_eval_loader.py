"""Inspect `.eval` archive loader, VM trace event hook, and ref/attachment resolver."""

from __future__ import annotations

import contextlib
import datetime
import json
import os
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Any

from capsem.trace import (
    DROPPED_SECURITY_TABLES,
    HARBOR_EVENT_TABLES,
    load_rows_tables,
    parse_iso_datetime,
)
from inspect_ai.log import read_eval_log

CAPSEM_EVENT_TABLES: tuple[str, ...] = (
    *HARBOR_EVENT_TABLES,
    "tool_calls",
    "tool_responses",
    "model_calls",
)


async def export_sample_vm_events(
    controller: Any, vm_id: str, *, labels: Mapping[str, str] | None = None
) -> None:
    """Export `<vm_id>.rows.ndjson.gz` + `<vm_id>.warc.gz` when `CAPSEM_TRACE_EXPORT_DIR` is set."""
    export_dir = os.environ.get("CAPSEM_TRACE_EXPORT_DIR", "").strip()
    if not export_dir:
        return
    vm_obj = getattr(controller, "_sessions", {}).get(vm_id)
    if vm_obj is None and callable(get_fn := getattr(controller, "get_vm_session", None)):
        with contextlib.suppress(Exception):
            vm_obj = get_fn(vm_id)
    if vm_obj is None:
        return
    export_api = getattr(vm_obj, "export", None)
    events_fn = getattr(export_api, "events", None)
    if not callable(events_fn):
        return
    quiesced = False
    if callable(quiesce_fn := getattr(export_api, "quiesce", None)):
        with contextlib.suppress(Exception):
            quiesced = bool(await quiesce_fn())
    if not quiesced and callable(stop_fn := getattr(vm_obj, "stop", None)):
        with contextlib.suppress(Exception):
            await stop_fn()
    with contextlib.suppress(Exception):
        await events_fn(export_dir, labels=labels, write_meta=True)


def parse_iso_epoch(ts: Any) -> float | None:
    """Convert an ISO-8601 string, datetime, or numeric epoch into a UTC POSIX timestamp."""
    if ts is None or isinstance(ts, (int, float)):
        return float(ts) if ts is not None else None
    return dt.timestamp() if (dt := parse_iso_datetime(ts)) is not None else None


def format_iso_utc(ts: Any) -> str | None:
    """Format an ISO-8601 string, datetime, or numeric epoch as `YYYY-MM-DDTHH:MM:SSZ`."""
    if (epoch := parse_iso_epoch(ts)) is None:
        return None
    iso = datetime.datetime.fromtimestamp(epoch, tz=datetime.UTC).isoformat()
    return iso.replace("+00:00", "Z")


def load_db_events(
    rows_path: str | Path,
    *,
    include_security_events: bool = False,
    tables: Sequence[str] | None = None,
) -> dict[str, list[tuple[float, dict[str, Any]]]]:
    """Load `(epoch, row_dict)` pairs from `.rows.ndjson.gz` event tables for window slicing."""
    sec = DROPPED_SECURITY_TABLES if include_security_events else ()
    eff_tables = tuple(tables) if tables is not None else CAPSEM_EVENT_TABLES + sec
    loaded: dict[str, list[tuple[float, dict[str, Any]]]] = {t: [] for t in eff_tables}
    for tbl, rows in load_rows_tables(rows_path, eff_tables).items():
        for row in rows:
            ep = parse_iso_epoch(row.get("timestamp"))
            if ep is None and row.get("timestamp_unix_ms") is not None:
                ep = float(row["timestamp_unix_ms"]) / 1000.0
            if ep is not None:
                loaded[tbl].append((ep, row))
    return loaded


load_rows_events = load_db_events


def slice_events(
    db_events: Mapping[str, Sequence[tuple[float, dict[str, Any]]]],
    start_epoch: float,
    end_epoch: float,
    *,
    pad_secs: float = 0.25,
) -> dict[str, list[dict[str, Any]]]:
    """Filter `db_events` to rows whose timestamp falls within `[start - pad, end + pad]`."""
    lo, hi = start_epoch - pad_secs, max(end_epoch, start_epoch) + pad_secs
    return {t: m for t, rows in db_events.items() if (m := [r for ep, r in rows if lo <= ep <= hi])}


def match_sample_db(
    db_cache: Mapping[Path, Mapping[str, Sequence[tuple[float, dict[str, Any]]]]],
    sample_start: float | None,
    sample_end: float | None,
    sample_cmds: set[str],
) -> Path | None:
    """Select the `.rows.ndjson.gz` in `db_cache` that best matches `[sample_start, sample_end]`."""
    if not db_cache:
        return None
    if sample_start is None or sample_end is None:
        return next(iter(db_cache)) if len(db_cache) == 1 else None
    best_db, best_score = None, 0
    for db_path, db_events in db_cache.items():
        sliced = slice_events(db_events, sample_start, sample_end, pad_secs=5.0)
        cmd_bonus = sum(
            1000
            for row in sliced.get("exec_events", [])
            if any(c in str(row.get("command") or "") for c in sample_cmds if c)
        )
        if (score := sum(len(v) for v in sliced.values()) + cmd_bonus) > best_score:
            best_score, best_db = score, db_path
    return best_db or (next(iter(db_cache)) if len(db_cache) == 1 else None)


def resolve_sample_refs_and_attachments(sample: dict[str, Any]) -> dict[str, Any]:
    """Resolve Inspect `input_refs`, `call_refs`, and `attachment://` pointers in `sample`."""
    raw_att, raw_ed = sample.get("attachments"), sample.get("events_data")
    attachments = raw_att if isinstance(raw_att, Mapping) else {}
    events_data = raw_ed if isinstance(raw_ed, Mapping) else {}
    raw_msgs, raw_calls = events_data.get("messages"), events_data.get("calls")
    messages_pool = raw_msgs if isinstance(raw_msgs, list) else []
    calls_pool = raw_calls if isinstance(raw_calls, list) else []

    for ev in (e for e in (sample.get("events") or []) if isinstance(e, dict)):
        in_refs = ev.get("input_refs")
        if isinstance(in_refs, list) and in_refs and not ev.get("input"):
            ev["input"] = [
                messages_pool[i]
                for span in in_refs
                if isinstance(span, (list, tuple)) and len(span) == 2
                for i in range(int(span[0]), int(span[1]))
                if 0 <= i < len(messages_pool)
            ]
        call_obj = ev.get("call")
        if isinstance(call_obj, dict):
            call_refs = call_obj.get("call_refs")
            call_key = str(call_obj.get("call_key") or "contents")
            req = call_obj.get("request")
            if (
                isinstance(req, dict)
                and isinstance(call_refs, list)
                and call_refs
                and not req.get(call_key)
            ):
                req[call_key] = [
                    calls_pool[i]
                    for span in call_refs
                    if isinstance(span, (list, tuple)) and len(span) == 2
                    for i in range(int(span[0]), int(span[1]))
                    if 0 <= i < len(calls_pool)
                ]

    if not attachments:
        return sample

    def _resolve(obj: Any) -> Any:
        if isinstance(obj, str):
            key = obj.removeprefix("attachment://") if obj.startswith("attachment://") else ""
            return attachments[key] if key and key in attachments else obj
        if isinstance(obj, list):
            return [_resolve(item) for item in obj]
        if isinstance(obj, dict):
            return {k: _resolve(v) for k, v in obj.items() if k != "attachments"}
        return obj

    resolved = _resolve(sample)
    if isinstance(resolved, dict):
        resolved["attachments"] = dict(attachments)
        return resolved
    return sample


def load_eval_log_dict(eval_log_path: str | Path) -> dict[str, Any]:
    """Load an Inspect `.eval` archive into a resolved dict via `inspect_ai.log.read_eval_log`."""
    path = Path(eval_log_path)
    if not path.is_file():
        raise FileNotFoundError(f"Inspect eval log not found: {path}")
    log_obj = read_eval_log(str(path), resolve_attachments="full")
    log_dict = (
        log_obj.model_dump(mode="json")
        if hasattr(log_obj, "model_dump")
        else json.loads(log_obj.model_dump_json())
    )
    if not isinstance(log_dict, dict):
        raise ValueError(f"Expected dict from read_eval_log({path}), got {type(log_dict).__name__}")
    if isinstance(samples := log_dict.get("samples"), list):
        log_dict["samples"] = [
            resolve_sample_refs_and_attachments(s) if isinstance(s, dict) else s for s in samples
        ]
    return log_dict


def discover_eval_files(paths: Sequence[str | Path]) -> list[Path]:
    """Expand file/directory arguments into a deduplicated sorted list of `.eval` archives."""
    found: list[Path] = []
    seen: set[Path] = set()
    for raw in paths:
        p = Path(raw)
        candidates = sorted(p.rglob("*.eval")) if p.is_dir() else ([p] if p.is_file() else [])
        for c in candidates:
            if (resolved := c.resolve()) not in seen:
                seen.add(resolved)
                found.append(c)
    return found
