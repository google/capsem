"""Export ATIF-v1.7 trajectories, LLM calls, and Capsem `rows.ndjson.gz` from Inspect `.eval`."""

from __future__ import annotations

import argparse
import contextlib
import importlib.util
import json
import logging
import os
import signal
import subprocess
import sys
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Any

from capsem.trace import resolve_include_security_events, validate_atif_trajectory

from inspect_capsem._eval_loader import (
    discover_eval_files,
    load_db_events,
    load_eval_log_dict,
)
from inspect_capsem._trace_convert import _convert_sample

logger = logging.getLogger(__name__)


def _env_include_security_events(explicit: bool | None) -> bool:
    return resolve_include_security_events(explicit)


def convert_eval_log(
    eval_log_path: str | Path,
    out_dir: str | Path,
    *,
    trace_db_dir: str | Path | None = None,
    include_security_events: bool | None = None,
) -> list[Path]:
    """Convert an Inspect `.eval` log (+ optional `.rows.ndjson.gz`) into ATIF-v1.7 files."""
    inc_sec = _env_include_security_events(include_security_events)
    log, dest_dir = load_eval_log_dict(eval_log_path), Path(out_dir)
    dest_dir.mkdir(parents=True, exist_ok=True)
    db_dir = Path(trace_db_dir) if trace_db_dir is not None else dest_dir
    db_paths = sorted(db_dir.glob("*.rows.ndjson.gz")) if db_dir.exists() else []
    if not db_paths and Path(eval_log_path).parent != db_dir:
        db_paths = sorted(Path(eval_log_path).parent.glob("*.rows.ndjson.gz"))
    db_cache = {p: load_db_events(p, include_security_events=inc_sec) for p in db_paths}

    eval_hdr = log.get("eval") or {}
    run_id = str(eval_hdr.get("run_id") or log.get("run_id") or "none")
    model = str(eval_hdr.get("model") or "unknown")
    raw_task = str(eval_hdr.get("task") or "unknown")
    pkg_ver = str((eval_hdr.get("packages") or {}).get("inspect_ai") or "0.1.0")
    return [
        _convert_sample(
            s,
            run_id=run_id,
            model=model,
            raw_task=raw_task,
            pkg_ver=pkg_ver,
            eval_hdr=eval_hdr,
            db_cache=db_cache,
            dest_dir=dest_dir,
        )
        for s in (log.get("samples") or [])
        if isinstance(s, Mapping)
    ]


convert_log = convert_eval_log


def _has_harbor_validator() -> bool:
    try:
        return importlib.util.find_spec("harbor.utils.trajectory_validator") is not None
    except (ModuleNotFoundError, ValueError):
        return False


def validate_trajectory_file(trajectory_path: str | Path) -> list[str]:
    """Validate an exported ATIF-v1.7 `.trajectory.json` file."""
    path = Path(trajectory_path)
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except Exception as exc:
        return [f"{path}: invalid JSON: {exc}"]
    if not isinstance(data, dict):
        return [f"{path}: root must be a JSON object"]
    errors = validate_atif_trajectory(data, label=str(path))
    if _has_harbor_validator():
        proc = subprocess.run(
            [sys.executable, "-m", "harbor.utils.trajectory_validator", str(path)],
            capture_output=True,
            text=True,
            check=False,
        )
        if proc.returncode != 0:
            detail = (proc.stderr or proc.stdout).strip()
            errors.append(f"{path}: harbor.utils.trajectory_validator failed: {detail}")
    return errors


def export_traces(
    eval_paths: Sequence[str | Path],
    out_dir: str | Path | None = None,
    *,
    trace_db_dir: str | Path | None = None,
    include_security_events: bool | None = None,
    validate: bool = False,
) -> dict[str, Any]:
    """Export ATIF-v1.7 trajectories for `.eval` files in `eval_paths` and return a summary."""
    eval_files = discover_eval_files(eval_paths)
    env_out = os.environ.get("CAPSEM_TRACE_EXPORT_DIR", "").strip()
    written_files: list[Path] = []
    validation_errors: list[str] = []
    request_keys, response_keys = set[str](), set[str]()
    total_agent_steps, total_llm_calls, samples_with_capsem_db = 0, 0, 0
    trajectory_coverage: list[dict[str, Any]] = []

    for ef in eval_files:
        target_out = (
            Path(out_dir)
            if out_dir is not None
            else (Path(env_out) if env_out else ef.parent / "traces")
        )
        for tp in convert_eval_log(
            ef,
            target_out,
            trace_db_dir=trace_db_dir,
            include_security_events=include_security_events,
        ):
            written_files.append(tp)
            traj = json.loads(tp.read_text(encoding="utf-8"))
            if (traj.get("extra") or {}).get("capsem_session_rows"):
                samples_with_capsem_db += 1
            t_agent, t_llm = 0, 0
            for step in traj.get("steps") or []:
                if step.get("source") == "agent":
                    total_agent_steps += 1
                    t_agent += 1
                    if isinstance(llm := (step.get("extra") or {}).get("llm_call"), Mapping):
                        total_llm_calls += 1
                        t_llm += 1
                        if isinstance(req := llm.get("request"), Mapping):
                            request_keys.update(str(k) for k in req)
                        if isinstance(resp := llm.get("response"), Mapping):
                            response_keys.update(str(k) for k in resp)
            cov = (traj.get("extra") or {}).get("llm_call_coverage") or {}
            trajectory_coverage.append(
                {
                    "trajectory_file": str(tp),
                    "session_id": traj.get("session_id"),
                    "agent_steps": t_agent,
                    "steps_with_llm_call": t_llm,
                    "steps_without_llm_call": t_agent - t_llm,
                    "errored_model_events_skipped": cov.get("errored_model_events_skipped", 0),
                }
            )
            if validate:
                validation_errors.extend(validate_trajectory_file(tp))

    without_llm = total_agent_steps - total_llm_calls
    return {
        "eval_files": [str(p) for p in eval_files],
        "trajectory_files": [str(p) for p in written_files],
        "total_samples": len(written_files),
        "total_agent_steps": total_agent_steps,
        "total_llm_calls": total_llm_calls,
        "steps_with_llm_call": total_llm_calls,
        "steps_without_llm_call": without_llm,
        "llm_call_coverage_ratio": round(total_llm_calls / total_agent_steps, 4)
        if total_agent_steps
        else 1.0,
        "trajectory_coverage": trajectory_coverage,
        "llm_request_keys": sorted(request_keys),
        "llm_response_keys": sorted(response_keys),
        "samples_with_capsem_db": samples_with_capsem_db,
        "validated": validate,
        "validation_errors": validation_errors,
    }


def _format_summary_line(summary: Mapping[str, Any]) -> str:
    with_llm = summary["steps_with_llm_call"]
    tot_steps = summary["total_agent_steps"]
    return (
        f"Exported {summary['total_samples']} ATIF-v1.7 trajectory file(s) from "
        f"{len(summary['eval_files'])} .eval log(s) ({with_llm}/{tot_steps} agent steps "
        f"with llm_call [{summary['steps_without_llm_call']} without], "
        f"{summary['samples_with_capsem_db']} with Capsem rows.ndjson.gz)."
    )


def run_with_signal_forwarding(
    cmd: Sequence[str],
    *,
    log_dir: str | Path,
    trace_dir: str | Path,
    include_security_events: bool | None = None,
    validate: bool = True,
) -> int:
    """Spawn `cmd`, forward signals to child, wait for exit, then run `export_traces`."""
    child_env = os.environ.copy()
    child_env.setdefault("INSPECT_EVAL_LOG_MODEL_API", "1")

    def _reset_child_signals() -> None:
        for s in (signal.SIGINT, signal.SIGQUIT, signal.SIGTERM):
            signal.signal(s, signal.SIG_DFL)

    proc = subprocess.Popen(
        list(cmd), env=child_env, preexec_fn=_reset_child_signals if hasattr(os, "fork") else None
    )

    def _forward_signal(signum: int, _frame: Any) -> None:
        if proc.poll() is None:
            with contextlib.suppress(ProcessLookupError, PermissionError, OSError):
                proc.send_signal(signum)

    prev_handlers: dict[int, Any] = {}
    for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGQUIT):
        with contextlib.suppress(ValueError, OSError):
            prev_handlers[sig] = signal.signal(sig, _forward_signal)

    rc, export_failed = 0, False
    try:
        while True:
            try:
                rc = proc.wait()
                break
            except InterruptedError:
                continue
    finally:
        for sig, prev in prev_handlers.items():
            with contextlib.suppress(ValueError, OSError):
                signal.signal(sig, prev)
        if str(log_dir).strip() and str(trace_dir).strip():
            try:
                res = export_traces(
                    [log_dir],
                    out_dir=trace_dir,
                    include_security_events=include_security_events,
                    validate=validate,
                )
                print(_format_summary_line(res))
                if res.get("validation_errors"):
                    for err in res["validation_errors"]:
                        logger.error("Trace validation failed: %s", err)
                    export_failed = True
            except Exception as exc:
                logger.error("Failed to export traces from %s to %s: %s", log_dir, trace_dir, exc)
                export_failed = True
    exit_code = 128 + (-rc) if rc < 0 else rc
    return exit_code if exit_code != 0 else (1 if export_failed else 0)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Export ATIF-v1.7 trajectories and Capsem rows.ndjson.gz events from .eval logs"
    )
    parser.add_argument("eval_paths", nargs="+", help="Inspect .eval files or directories")
    parser.add_argument("--out-dir", "-o", type=Path, default=None)
    parser.add_argument("--trace-db-dir", type=Path, default=None)
    parser.add_argument("--include-security-events", action="store_true", default=None)
    parser.add_argument("--validate", action="store_true")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)

    summary = export_traces(
        args.eval_paths,
        out_dir=args.out_dir,
        trace_db_dir=args.trace_db_dir,
        include_security_events=args.include_security_events,
        validate=args.validate,
    )
    print(json.dumps(summary, indent=2) if args.json else _format_summary_line(summary))
    if summary["validation_errors"]:
        for err in summary["validation_errors"]:
            print(f"ERROR: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
