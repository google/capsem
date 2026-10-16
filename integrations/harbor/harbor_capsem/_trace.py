"""Export Capsem `rows.ndjson.gz` + `warc.gz` and enrich or synthesize `trajectory.json`."""

from __future__ import annotations

import asyncio
import contextlib
import json
import logging
import os
from pathlib import Path
from typing import Any

from capsem import CapsemError
from capsem import trace as _sdk_trace
from capsem.trace import (
    HARBOR_EVENT_TABLES,
    BodiesMode,
    BodyReader,
    WarcBodyReader,
    build_steps_from_session_tables,
    has_session_events,
    hydrate_tables_from_bodies,
    load_rows_tables,
    resolve_include_security_events,
    resolve_warc_path,
)
from harbor.models.trial.paths import TrialPaths
from harbor.utils.trajectory_validator import TrajectoryValidator

from harbor_capsem._credentials import coerce_bool_flag

logger = logging.getLogger(__name__)
_EVENT_TABLES = HARBOR_EVENT_TABLES


def resolve_controller_vm(controller: Any, vm_id: str) -> Any:
    """Resolve a `capsem.VM` handle for `vm_id` from `controller` without reading `_sessions`."""
    if callable(get_vm := getattr(controller, "get_vm_session", None)):
        return get_vm(vm_id)
    hv = getattr(controller, "hypervisor", None) or getattr(controller, "_hypervisor", None)
    return vm_fn(id=vm_id) if callable(vm_fn := getattr(hv, "vm", None)) else None


def _resolve_trial_agent_info(
    rows_path: Path,
    *,
    trial_dir: Path | None = None,
    agent_name: str | None = None,
    agent_version: str | None = None,
) -> tuple[str | None, str | None]:
    name, ver = agent_name, agent_version
    for base in dict.fromkeys(p for p in (trial_dir, rows_path.parent) if p is not None):
        for fname in ("result.json", "lock.json", "config.json"):
            if (name and ver) or not (fpath := base / fname).is_file():
                continue
            with contextlib.suppress(Exception):
                if not isinstance(meta := json.loads(fpath.read_text(encoding="utf-8")), dict):
                    continue
                info = meta.get("agent_info") if isinstance(meta.get("agent_info"), dict) else {}
                acfg = meta.get("agent") if isinstance(meta.get("agent"), dict) else {}
                name = name or info.get("name") or acfg.get("name")
                if not name and fname == "config.json" and "task" in meta and "trial_name" in meta:
                    name = "oracle"
                ver = ver or info.get("version") or acfg.get("version")
    if name and (not ver or ver == "unknown") and rows_path.is_file():
        if name == "oracle":
            ver = "1.0.0"
        else:
            for row in load_rows_tables(rows_path, ("exec_events",))["exec_events"]:
                cmd = str(row.get("command") or "")
                out = str(row.get("stdout_preview") or row.get("stdout") or "")
                if f"{name} --version" in cmd and row.get("exit_code") == 0 and out.strip():
                    line = [ln.strip() for ln in out.splitlines() if ln.strip()][-1]
                    ver = line.removeprefix(name).strip() or line
                    break
    return name, (ver or "unknown") if name else ver


def _attach_mc_previews(
    traj: dict[str, Any], rows_path: Path, *, warc_path: Path | None = None
) -> None:
    if not rows_path.is_file():
        return
    tbls = load_rows_tables(rows_path, ("model_calls", "net_events", "event_body_blobs"))
    if (wp := resolve_warc_path(rows_path, warc_path)) is not None:
        hydrate_tables_from_bodies(tbls, WarcBodyReader(wp), full=True)
    if (mcalls := tbls["model_calls"]) and isinstance(
        fm := traj.setdefault("final_metrics", {}), dict
    ):
        fm.setdefault("total_prompt_tokens", sum(int(m.get("input_tokens") or 0) for m in mcalls))
        fm.setdefault(
            "total_completion_tokens", sum(int(m.get("output_tokens") or 0) for m in mcalls)
        )
        fm.setdefault("total_steps", len(traj.get("steps") or ()))
    by_id = {r.get("id"): r for r in tbls["model_calls"]}
    by_turn = {str(r.get("turn_id") or ""): r for r in tbls["model_calls"] if r.get("turn_id")}
    net_by_turn = {
        str(r.get("turn_id") or ""): r
        for r in tbls["net_events"]
        if r.get("turn_id") and (r.get("response_body_preview") or r.get("request_body_preview"))
    }
    for step in traj.get("steps") or ():
        mc_ext = (step.get("extra") or {}).get("capsem_model_call")
        recs = [mc_ext] if isinstance(mc_ext, dict) else (mc_ext or ())
        for rec in (r for r in recs if isinstance(r, dict)):
            tid = str(rec.get("turn_id") or "")
            src = by_id.get(rec.get("id")) or by_turn.get(tid) or {}
            net = net_by_turn.get(tid) or {}
            req_p = (
                src.get("request_body_preview")
                or src.get("request_body")
                or net.get("request_body_preview")
            )
            resp_p = (
                src.get("response_body_preview")
                or src.get("response_body")
                or net.get("response_body_preview")
                or src.get("text_content")
                or src.get("thinking_content")
            )
            rec.setdefault("request_body_preview", req_p)
            rec.setdefault("response_body_preview", resp_p)


def _validate_and_write(
    trajectory_data: dict[str, Any], trajectory_path: Path | None = None
) -> dict[str, Any]:
    validator = TrajectoryValidator()
    if not validator.validate(trajectory_data, validate_images=False):
        raise ValueError(f"ATIF trajectory failed validation: {validator.get_errors()}")
    if trajectory_path is not None:
        trajectory_path.parent.mkdir(parents=True, exist_ok=True)
        trajectory_path.write_text(json.dumps(trajectory_data, indent=2) + "\n", encoding="utf-8")
    return trajectory_data


def resolve_trace_options(
    *, export_trace: Any = None, include_security_events: Any = None
) -> tuple[bool, bool]:
    """Resolve `export_trace` and `include_security_events` from kwargs or `CAPSEM_*` env."""
    raw_exp = os.environ.get("CAPSEM_TRACE_EXPORT", "") if export_trace is None else export_trace
    eff_inc_sec = (
        coerce_bool_flag(include_security_events, default=False)
        if include_security_events is not None
        else resolve_include_security_events(None)
    )
    return coerce_bool_flag(raw_exp, default=False), eff_inc_sec


def events_to_atif(
    rows_path: Path,
    *,
    vm_id: str | None = None,
    session_id: str | None = None,
    agent_name: str | None = None,
    agent_version: str | None = None,
    warc_path: Path | None = None,
    include_security_events: bool = False,
    bodies: BodiesMode = "warc",
    body_reader: BodyReader | None = None,
    output_path: Path | None = None,
) -> dict[str, Any]:
    """Synthesize a Harbor-validated `ATIF-v1.7` trajectory from `rows.ndjson.gz` + `warc.gz`."""
    eff_vm_id = vm_id or rows_path.name.removesuffix(".rows.ndjson.gz") or rows_path.stem
    name, ver = _resolve_trial_agent_info(
        rows_path, agent_name=agent_name, agent_version=agent_version
    )
    kw: dict[str, Any] = {"vm_id": eff_vm_id, "session_id": session_id, "agent_name": name}
    kw.update({"agent_version": ver, "warc_path": warc_path, "bodies": bodies})
    kw.update({"include_security_events": include_security_events, "body_reader": body_reader})
    traj = _sdk_trace.rows_to_atif(rows_path, **kw)
    _attach_mc_previews(traj, rows_path, warc_path=warc_path)
    return _validate_and_write(traj, output_path)


rows_to_atif = events_to_atif


def enrich_trajectory_file(
    trajectory_path: Path,
    rows_path: Path,
    *,
    vm_id: str,
    warc_path: Path | None = None,
    include_security_events: bool = False,
    bodies: BodiesMode = "warc",
    body_reader: BodyReader | None = None,
) -> dict[str, Any]:
    """Enrich an ATIF `trajectory.json` file in place from `rows_path` and validate it."""
    traj: dict[str, Any] = json.loads(trajectory_path.read_text(encoding="utf-8"))
    orig_ver, traj["schema_version"] = traj.get("schema_version"), "ATIF-v1.7"
    kw: dict[str, Any] = {"vm_id": vm_id, "warc_path": warc_path, "bodies": bodies}
    kw.update({"include_security_events": include_security_events, "body_reader": body_reader})
    traj = _sdk_trace.enrich_atif_trajectory(traj, rows_path, **kw)
    if orig_ver:
        traj["schema_version"] = orig_ver
    _attach_mc_previews(traj, rows_path, warc_path=warc_path)
    return _validate_and_write(traj, trajectory_path)


async def export_and_enrich_trial_trace(
    *,
    vm_id: str,
    trial_paths: TrialPaths,
    session_id: str | None = None,
    agent_name: str | None = None,
    agent_version: str | None = None,
    include_security_events: bool = False,
    vm: Any = None,
    bodies: BodiesMode | None = None,
) -> tuple[Path | None, Path | None]:
    """Export `<vm_id>.rows.ndjson.gz` + `<vm_id>.warc.gz` and update `trajectory.json`."""
    traj_path, tdir = trial_paths.agent_dir / "trajectory.json", trial_paths.trial_dir
    dst_rows: Path | None = None
    dst_warc: Path | None = None
    events_fn = getattr(getattr(vm, "export", None), "events", None) if vm is not None else None
    if callable(events_fn):
        with contextlib.suppress(CapsemError):
            dst_rows, dst_warc = await events_fn(
                tdir, include_security_events=include_security_events
            )
    if dst_rows is None:
        cand = tdir / f"{vm_id}.rows.ndjson.gz"
        if not cand.is_file():
            return None, None
        dst_rows = cand
        dst_warc = resolve_warc_path(dst_rows)
    rows_file: Path = dst_rows

    def _run_sync() -> tuple[Path | None, Path | None]:
        eff_b: BodiesMode = bodies or "warc"
        kw: dict[str, Any] = {"vm_id": vm_id, "warc_path": dst_warc, "bodies": eff_b}
        kw["include_security_events"] = include_security_events
        if traj_path.is_file():
            enrich_trajectory_file(traj_path, rows_file, **kw)
        elif has_session_events(rows_file):
            kw.update({"session_id": session_id, "agent_name": agent_name})
            kw.update({"agent_version": agent_version, "output_path": traj_path})
            events_to_atif(rows_file, **kw)
        return rows_file, dst_warc

    return await asyncio.to_thread(_run_sync)


async def stop_and_export_trial_trace(
    controller: Any,
    vm_id: str,
    *,
    trial_paths: TrialPaths,
    session_id: str | None,
    include_security_events: bool = False,
    strict: bool = False,
) -> None:
    """Quiesce `vm_id` and export its session events and ATIF trajectory (best-effort)."""
    vm_obj = resolve_controller_vm(controller, vm_id)
    try:
        q_fn = getattr(getattr(vm_obj, "export", None), "quiesce", None)
        quiesced = bool(await q_fn()) if callable(q_fn) else False
        if not quiesced and callable(stop_fn := getattr(vm_obj, "stop", None)):
            await stop_fn()
        kw: dict[str, Any] = {"vm_id": vm_id, "trial_paths": trial_paths, "session_id": session_id}
        await export_and_enrich_trial_trace(
            include_security_events=include_security_events, vm=vm_obj, **kw
        )
    except Exception as exc:
        if strict:
            raise
        logger.warning("Best-effort Capsem trace export failed for %s: %s", vm_id, exc)


__all__ = [
    "_EVENT_TABLES",
    "_validate_and_write",
    "build_steps_from_session_tables",
    "enrich_trajectory_file",
    "events_to_atif",
    "export_and_enrich_trial_trace",
    "resolve_controller_vm",
    "resolve_trace_options",
    "rows_to_atif",
    "stop_and_export_trial_trace",
]
