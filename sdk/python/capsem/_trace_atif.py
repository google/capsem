"""ATIF-v1.7 step synthesis from Capsem ledger rows (`model_calls`, `tool_calls`, `exec_events`)."""

from __future__ import annotations

import json
import re
from collections.abc import Mapping, Sequence
from typing import Any

from ._trace_enrich import (
    _mc_summary,
    attach_capsem_summary,
    enrich_steps_by_timestamp_window,
    enrich_steps_by_turn_id,
    validate_atif_trajectory,
)

_TRUNC_TEXT_RE = re.compile(r'"text"\s*:\s*"((?:[^"\\]|\\.)*)')


def _opt_int(value: Any) -> int | None:
    if value is None:
        return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _parse_json_obj(raw: Any) -> dict[str, Any]:
    if isinstance(raw, dict):
        return raw
    if isinstance(raw, str) and raw.strip():
        try:
            parsed = json.loads(raw)
        except ValueError:
            return {"_raw": raw}
        return parsed if isinstance(parsed, dict) else {"value": parsed}
    return {}


def _extract_text(content: Any) -> str:
    if isinstance(content, str):
        return content
    if isinstance(content, Mapping):
        if isinstance(content.get("text"), str):
            return str(content["text"])
        if isinstance(content.get("parts"), Sequence):
            return _extract_text(content["parts"])
        if isinstance(content.get("content"), (str, Sequence, Mapping)):
            return _extract_text(content["content"])
    if isinstance(content, Sequence) and not isinstance(content, (str, bytes)):
        parts: list[str] = []
        for item in content:
            if isinstance(item, str) and item:
                parts.append(item)
            elif (
                isinstance(item, Mapping)
                and item.get("type") not in ("tool_use", "tool_result")
                and (text := str(item.get("text", "")))
            ):
                parts.append(text)
        return "\n".join(parts)
    return ""


def _extract_initial_prompts(mc: Mapping[str, Any]) -> tuple[str | None, str | None]:
    sys_msg: str | None = str(mc.get("system_prompt_preview") or "").strip() or None
    user_msg: str | None = None
    raw_req = mc.get("request_body") or mc.get("request_body_preview")
    if not isinstance(raw_req, str) or not raw_req.strip():
        return sys_msg, None
    try:
        src = json.loads(raw_req)
    except ValueError:
        m = _TRUNC_TEXT_RE.search(raw_req)
        if not m:
            return sys_msg, None
        raw_lit = re.sub(r"\\(?:u[0-9a-fA-F]{0,3})?$", "", m.group(1))
        try:
            return sys_msg, str(json.loads(f'"{raw_lit}"')).strip()
        except ValueError:
            return sys_msg, raw_lit.strip()
    if not isinstance(src, Mapping):
        return sys_msg, None
    for k in ("system", "instructions", "systemInstruction", "system_instruction"):
        if k in src and (extracted := _extract_text(src[k]).strip()):
            sys_msg = extracted
            break
    if isinstance(src.get("input"), str):
        user_msg = _extract_text(src["input"]).strip() or None
    for seq in (src.get("messages"), src.get("input"), src.get("contents")):
        if not isinstance(seq, Sequence) or isinstance(seq, (str, bytes)):
            continue
        for item in seq:
            if not isinstance(item, Mapping):
                continue
            role = str(item.get("role") or "").lower()
            if role in ("system", "developer") and not sys_msg:
                sys_msg = _extract_text(item.get("content") or item.get("parts"))
            elif role == "user" and (
                u_txt := _extract_text(item.get("content") or item.get("parts") or item.get("text"))
            ):
                user_msg = u_txt
    return (sys_msg.strip() if sys_msg else None), (user_msg.strip() if user_msg else None)


def _pair_tool_response(
    tc_id: str,
    fn: str,
    idx: int,
    mc_id: int,
    next_mc_id: int | None,
    tresp_by_cid: Mapping[str, list[tuple[int, Mapping[str, Any]]]],
    tresp_with_idx: Sequence[tuple[int, Mapping[str, Any]]],
    call_scopes: Sequence[Sequence[tuple[int, Mapping[str, Any]]]],
    used_tresp: set[int],
) -> tuple[int, Mapping[str, Any]] | None:
    for r_idx, r in tresp_by_cid.get(tc_id, ()):
        if r_idx not in used_tresp:
            return r_idx, r
    fn_scopes = [
        [
            x
            for x in tresp_with_idx
            if x[0] not in used_tresp and _opt_int(x[1].get("model_call_id")) == t
        ]
        for t in (next_mc_id, mc_id)
        if t is not None
    ]
    fn_scopes.append([x for x in tresp_with_idx if x[0] not in used_tresp])
    idx_pat = re.compile(rf"^{re.escape(fn)}_(?:\d+_)*{idx}$") if fn else None
    any_pat = re.compile(rf"^{re.escape(fn)}_\d+(?:_\d+)*$") if fn else None
    for scope in fn_scopes:
        if not fn:
            break
        for r_idx, r in scope:
            cid = str(r.get("call_id") or "")
            if cid == fn or (idx_pat is not None and idx_pat.fullmatch(cid) is not None):
                return r_idx, r
        for r_idx, r in scope:
            cid = str(r.get("call_id") or "")
            if any_pat is not None and any_pat.fullmatch(cid) is not None:
                return r_idx, r
    for call_scope in call_scopes:
        if idx < len(call_scope) and call_scope[idx][0] not in used_tresp:
            return call_scope[idx]
        for r_idx, r in call_scope:
            if r_idx not in used_tresp:
                return r_idx, r
    return None


def build_steps_from_session_tables(
    model_calls: Sequence[Mapping[str, Any]],
    model_items: Sequence[Mapping[str, Any]],
    tool_calls: Sequence[Mapping[str, Any]],
    tool_responses: Sequence[Mapping[str, Any]],
    *,
    include_llm_call: bool = False,
) -> list[dict[str, Any]]:
    """Synthesize ATIF-v1.7 steps from `model_calls`, `model_items`, and tool tables."""
    items_by_mc: dict[int, list[Mapping[str, Any]]] = {}
    for item in model_items:
        if (mc_key := _opt_int(item.get("model_call_id"))) is not None:
            items_by_mc.setdefault(mc_key, []).append(item)
    tcalls_by_mc: dict[int, list[Mapping[str, Any]]] = {}
    for tc in tool_calls:
        if (mc_key := _opt_int(tc.get("model_call_id"))) is not None:
            tcalls_by_mc.setdefault(mc_key, []).append(tc)
    tresp_with_idx = list(enumerate(tool_responses))
    tresp_by_cid: dict[str, list[tuple[int, Mapping[str, Any]]]] = {}
    for idx, tr in tresp_with_idx:
        if cid := str(tr.get("call_id") or ""):
            tresp_by_cid.setdefault(cid, []).append((idx, tr))
    used_tresp: set[int] = set()
    steps: list[dict[str, Any]] = []
    step_id = 1
    if model_calls:
        first_mc = model_calls[0]
        first_ts = first_mc.get("timestamp")
        sys_prompt, user_prompt = _extract_initial_prompts(first_mc)
        if not user_prompt:
            first_items = items_by_mc.get(_opt_int(first_mc.get("id")) or 0, ())
            user_prompt = next(
                (
                    str(i["content"]).strip()
                    for i in first_items
                    if i.get("kind") in ("user", "input_text", "prompt", "request")
                    and str(i.get("content") or "").strip()
                ),
                None,
            )
        if sys_prompt:
            sys_step = {"step_id": step_id, "timestamp": first_ts}
            steps.append({**sys_step, "source": "system", "message": sys_prompt})
            step_id += 1
        if user_prompt or not steps:
            user_msg = user_prompt or "(Capsem VM session)"
            steps.append(
                {"step_id": step_id, "timestamp": first_ts, "source": "user", "message": user_msg}
            )
            step_id += 1

    for mc_idx, mc in enumerate(model_calls):
        mc_id = _opt_int(mc.get("id")) or 0
        next_mc = model_calls[mc_idx + 1] if mc_idx + 1 < len(model_calls) else None
        next_mc_id = _opt_int(next_mc.get("id")) if next_mc is not None else None
        items, tcalls = items_by_mc.get(mc_id, []), tcalls_by_mc.get(mc_id, [])
        msg = str(mc.get("text_content") or "") or "\n".join(
            str(i.get("content") or "")
            for i in items
            if i.get("kind") in ("text", "output_text", "message", "response") and i.get("content")
        )
        reasoning = str(mc.get("thinking_content") or "") or "\n".join(
            str(i.get("content") or "")
            for i in items
            if i.get("kind") in ("thinking", "reasoning", "thought") and i.get("content")
        )
        extra: dict[str, Any] = {
            "turn_id": mc.get("turn_id"),
            "capsem_model_call_id": mc_id,
            "capsem_model_call": _mc_summary(mc),
            **({"truncated": True} if mc.get("truncated") else {}),
        }
        if include_llm_call and (mc.get("request_body") or mc.get("response_body")):
            extra["llm_call"] = {
                "request": _parse_json_obj(mc.get("request_body")),
                "response": _parse_json_obj(mc.get("response_body")),
            }
        step: dict[str, Any] = {
            "step_id": step_id,
            "timestamp": mc.get("timestamp"),
            "source": "agent",
            "message": msg,
            "model_name": mc.get("model"),
            "llm_call_count": 1,
            "metrics": {
                "prompt_tokens": int(mc.get("input_tokens") or 0),
                "completion_tokens": int(mc.get("output_tokens") or 0),
                "cost_usd": mc.get("estimated_cost_usd"),
            },
            "extra": extra,
        }
        if reasoning:
            step["reasoning_content"] = reasoning
        if tcalls:
            call_scopes = [
                [x for x in tresp_with_idx if _opt_int(x[1].get("model_call_id")) == t]
                for t in (next_mc_id, mc_id)
                if t is not None
            ]
            atif_calls: list[dict[str, Any]] = []
            obs_results: list[dict[str, Any]] = []
            for idx, tc in enumerate(tcalls):
                tc_id = str(tc.get("call_id") or f"call_{mc_id}_{idx}")
                fn = str(tc.get("tool_name") or "unknown")
                args = _parse_json_obj(tc.get("arguments"))
                atif_calls.append({"tool_call_id": tc_id, "function_name": fn, "arguments": args})
                matched = _pair_tool_response(
                    tc_id,
                    fn,
                    idx,
                    mc_id,
                    next_mc_id,
                    tresp_by_cid,
                    tresp_with_idx,
                    call_scopes,
                    used_tresp,
                )
                if matched is not None:
                    used_tresp.add(matched[0])
                    r = matched[1]
                    content = str(r.get("content_full") or r.get("content_preview") or "")
                    res_obj: dict[str, Any] = {"source_call_id": tc_id, "content": content}
                    if r.get("is_error"):
                        res_obj["extra"] = {"is_error": True}
                    obs_results.append(res_obj)
                elif tc.get("response_preview") is not None:
                    obs_results.append(
                        {"source_call_id": tc_id, "content": str(tc["response_preview"])}
                    )
            step["tool_calls"] = atif_calls
            if obs_results:
                step["observation"] = {"results": obs_results}
        steps.append(step)
        step_id += 1
    return steps


__all__ = [
    "attach_capsem_summary",
    "build_steps_from_session_tables",
    "enrich_steps_by_timestamp_window",
    "enrich_steps_by_turn_id",
    "validate_atif_trajectory",
]
