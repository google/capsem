"""Sample-level ATIF-v1.7 step conversion and ModelEvent pairing for `inspect_capsem`."""

from __future__ import annotations

import contextlib
import json
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Any

from inspect_capsem._eval_loader import (
    format_iso_utc,
    match_sample_db,
    parse_iso_epoch,
    slice_events,
)


def _text_of(content: Any) -> str:
    if not content:
        return ""
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(
            str(c.get("text", ""))
            for c in content
            if isinstance(c, Mapping) and c.get("type") == "text"
        )
    return str(content)


def _reasoning_of(content: Any) -> str | None:
    if not isinstance(content, list):
        return None
    parts = [
        str(c.get("reasoning", ""))
        for c in content
        if isinstance(c, Mapping) and c.get("type") == "reasoning" and c.get("reasoning")
    ]
    return "\n".join(parts) or None


def _is_errored_model_event(me: Mapping[str, Any]) -> bool:
    """Return True if a ModelEvent failed with an API/retry error and produced no assistant turn."""
    if me.get("error"):
        return True
    out = me.get("output")
    if isinstance(out, Mapping):
        if out.get("error"):
            return True
        choices = out.get("choices")
        if (
            isinstance(choices, list)
            and choices
            and isinstance(choices[0], Mapping)
            and choices[0].get("stop_reason") == "error"
        ):
            return True
    return False


def _model_event_message_id(me: Mapping[str, Any]) -> str | None:
    """Extract `output.message.id` or `output.choices[0].message.id` from a ModelEvent."""
    out = me.get("output")
    if not isinstance(out, Mapping):
        return None
    if isinstance(msg := out.get("message"), Mapping) and msg.get("id"):
        return str(msg["id"])
    choices = out.get("choices")
    if (
        isinstance(choices, list)
        and choices
        and isinstance(choices[0], Mapping)
        and isinstance(c_msg := choices[0].get("message"), Mapping)
        and c_msg.get("id")
    ):
        return str(c_msg["id"])
    return None


def _enrich_agent_step_from_model_event(step: dict[str, Any], me: Mapping[str, Any]) -> None:
    if iso_ts := format_iso_utc(me.get("timestamp")):
        step["timestamp"] = iso_ts
    usage = (me.get("output") or {}).get("usage") or {}
    key_map = (
        ("input_tokens", "prompt_tokens"),
        ("output_tokens", "completion_tokens"),
        ("input_tokens_cache_read", "cached_tokens"),
    )
    metrics: dict[str, Any] = {
        k2: int(usage[k1]) for k1, k2 in key_map if usage.get(k1) is not None
    }
    if usage.get("reasoning_tokens") is not None:
        metrics["extra"] = {"reasoning_tokens": int(usage["reasoning_tokens"])}
    if metrics:
        step["metrics"] = metrics
    call_obj = me.get("call") or {}
    if isinstance(call_obj, Mapping) and (call_obj.get("request") or call_obj.get("response")):
        step.setdefault("extra", {})["llm_call"] = {
            k: call_obj[k] for k in ("request", "response") if call_obj.get(k)
        }


def _pair_model_event(
    msg_id: str,
    valid_mevs: Sequence[Mapping[str, Any]],
    mev_by_msg_id: dict[str, tuple[int, Mapping[str, Any]]],
    used_indices: set[int],
    fallback_idx: int,
) -> tuple[Mapping[str, Any] | None, int]:
    if msg_id and msg_id in mev_by_msg_id:
        matched_idx, matched_mev = mev_by_msg_id.pop(msg_id)
        used_indices.add(matched_idx)
        return matched_mev, fallback_idx
    while fallback_idx < len(valid_mevs):
        cand_idx = fallback_idx
        fallback_idx += 1
        if cand_idx in used_indices:
            continue
        cand_mev = valid_mevs[cand_idx]
        cand_mid = _model_event_message_id(cand_mev)
        if msg_id and cand_mid and cand_mid in mev_by_msg_id:
            continue
        used_indices.add(cand_idx)
        return cand_mev, fallback_idx
    return None, fallback_idx


def _convert_sample(
    sample: Mapping[str, Any],
    *,
    run_id: str,
    model: str,
    raw_task: str,
    pkg_ver: str,
    eval_hdr: Mapping[str, Any],
    db_cache: Mapping[Path, dict[str, list[tuple[float, dict[str, Any]]]]],
    dest_dir: Path,
) -> Path:
    tool_windows: dict[str, tuple[float, float]] = {}
    model_events: list[Mapping[str, Any]] = []
    sample_cmds: set[str] = set()
    epochs: list[float] = []

    for ev in (e for e in (sample.get("events") or []) if isinstance(e, Mapping)):
        t_s, t_e = parse_iso_epoch(ev.get("timestamp")), parse_iso_epoch(ev.get("completed"))
        t_e = t_e or t_s
        epochs.extend(x for x in (t_s, t_e) if x is not None)
        kind = ev.get("event")
        if kind == "tool" and ev.get("id") and t_s is not None and t_e is not None:
            tool_windows[str(ev["id"])] = (t_s, t_e)
            if isinstance(args := ev.get("arguments"), Mapping):
                cmd_raw = args.get("cmd") or args.get("command") or args.get("input") or ""
                if cmd := str(cmd_raw).strip():
                    sample_cmds.add(cmd[:80])
        elif kind == "model":
            model_events.append(ev)
        elif kind == "sandbox" and ev.get("cmd"):
            sample_cmds.add(str(ev["cmd"]).strip()[:80])

    valid_mevs = [ev for ev in model_events if not _is_errored_model_event(ev)]
    mev_by_msg_id: dict[str, tuple[int, Mapping[str, Any]]] = {}
    for idx, mev in enumerate(valid_mevs):
        if mid := _model_event_message_id(mev):
            mev_by_msg_id.setdefault(mid, (idx, mev))
    used_indices: set[int] = set()
    fallback_idx = 0

    best_db = match_sample_db(
        db_cache, min(epochs) if epochs else None, max(epochs) if epochs else None, sample_cmds
    )
    best_events = db_cache[best_db] if best_db is not None else {}
    steps: list[dict[str, Any]] = []
    pending: dict[str, Any] | None = None
    any_tool_joined = False

    for m in (msg for msg in (sample.get("messages") or []) if isinstance(msg, Mapping)):
        role = m.get("role")
        if role in ("system", "user"):
            pending = None
            steps.append(
                {"step_id": len(steps) + 1, "source": role, "message": _text_of(m.get("content"))}
            )
        elif role == "assistant":
            step: dict[str, Any] = {
                "step_id": len(steps) + 1,
                "source": "agent",
                "model_name": str(m.get("model") or model),
                "message": _text_of(m.get("content")),
                "llm_call_count": 1,
            }
            if reasoning := _reasoning_of(m.get("content")):
                step["reasoning_content"] = reasoning
            if tcs := m.get("tool_calls"):
                step["tool_calls"] = [
                    {
                        "tool_call_id": str(tc.get("id") or ""),
                        "function_name": str(tc.get("function") or ""),
                        "arguments": dict(tc.get("arguments") or {}),
                    }
                    for tc in tcs
                    if isinstance(tc, Mapping)
                ]
            matched_mev, fallback_idx = _pair_model_event(
                str(m.get("id") or "").strip(),
                valid_mevs,
                mev_by_msg_id,
                used_indices,
                fallback_idx,
            )
            if matched_mev is not None:
                _enrich_agent_step_from_model_event(step, matched_mev)
            steps.append(step)
            pending = step
        elif role == "tool":
            tc_id = str(m.get("tool_call_id") or "")
            res: dict[str, Any] = {"source_call_id": tc_id, "content": _text_of(m.get("content"))}
            if err := m.get("error"):
                res.setdefault("extra", {})["error"] = err
            if (
                tc_id in tool_windows
                and best_events
                and (joined := slice_events(best_events, *tool_windows[tc_id]))
            ):
                res.setdefault("extra", {})["capsem_events"] = joined
                any_tool_joined = True
            if pending is not None:
                pending.setdefault("observation", {"results": []})["results"].append(res)

    if (
        not any_tool_joined
        and best_events
        and pending is not None
        and epochs
        and (joined := slice_events(best_events, min(epochs), max(epochs), pad_secs=2.0))
    ):
        pending.setdefault("extra", {})["capsem_events"] = joined

    sample_id, epoch = sample.get("id"), sample.get("epoch", 1)
    meta_info, warc_name = None, None
    if best_db is not None:
        stem = best_db.name.removesuffix(".rows.ndjson.gz")
        if (mf := best_db.with_name(f"{stem}.meta.json")).exists():
            with contextlib.suppress(OSError, json.JSONDecodeError):
                meta_info = json.loads(mf.read_text(encoding="utf-8"))
        if (wf := best_db.with_name(f"{stem}.warc.gz")).exists():
            warc_name = wf.name

    agent_steps = [s for s in steps if s.get("source") == "agent"]
    with_llm = sum(
        1 for s in agent_steps if isinstance((s.get("extra") or {}).get("llm_call"), Mapping)
    )
    traj = {
        "schema_version": "ATIF-v1.7",
        "session_id": f"{run_id}/{sample_id}/{epoch}",
        "agent": {
            "name": f"inspect:{eval_hdr.get('solver') or 'agent'}",
            "version": pkg_ver,
            "model_name": model,
        },
        "steps": steps,
        "final_metrics": {
            "total_prompt_tokens": sum(
                (s.get("metrics") or {}).get("prompt_tokens", 0) for s in steps
            ),
            "total_completion_tokens": sum(
                (s.get("metrics") or {}).get("completion_tokens", 0) for s in steps
            ),
            "total_cached_tokens": sum(
                (s.get("metrics") or {}).get("cached_tokens", 0) for s in steps
            ),
            "total_steps": len(steps),
        },
        "extra": {
            "task": raw_task,
            "sample_id": sample_id,
            "epoch": epoch,
            "scores": sample.get("scores"),
            "sandbox": eval_hdr.get("sandbox"),
            "capsem_session_rows": best_db.name if best_db else None,
            "capsem_session_warc": warc_name,
            "capsem_session_db": best_db.name if best_db else None,
            "capsem_session_bodies": warc_name,
            "capsem_session_meta": meta_info,
            "llm_call_coverage": {
                "agent_steps": len(agent_steps),
                "steps_with_llm_call": with_llm,
                "steps_without_llm_call": len(agent_steps) - with_llm,
                "errored_model_events_skipped": len(model_events) - len(valid_mevs),
            },
        },
    }
    safe_id = str(sample_id).replace("/", "_")
    dest = dest_dir / f"{raw_task.replace('/', '_')}_{safe_id}_epoch{epoch}.trajectory.json"
    dest.write_text(json.dumps(traj, indent=2, default=str), encoding="utf-8")
    return dest
