"""Compose helpers derived from Pierre Tholoniat's bb61fc82d44bc42c978c4acf5435a1975600c75e."""

from __future__ import annotations

import re
from collections.abc import Mapping

from .compose_inputs import Fragments, InterpolationBudget

_BRACED_VAR_HEAD_RE = re.compile(
    r"^([A-Za-z_][A-Za-z0-9_]*)(?:(:-|-|:\?|\?|:\+|\+)(.*))?$", re.DOTALL
)

_BARE_VAR_RE = re.compile(r"([A-Za-z_][A-Za-z0-9_]*)")

_INLINE_COMMENT_RE = re.compile(r" +#.*$")

_DOUBLE_QUOTE_ESCAPES = {'"': '"', "\\": "\\", "$": "$$", "n": "\n", "r": "\r", "t": "\t"}

_QUOTED_DOTENV_TAIL_RE = re.compile(r"^[A-Za-z0-9_.\-]+$")


def _validate_quoted_dotenv_tail(
    tail: str,
) -> None:
    t = tail.strip()
    if t and not t.startswith("#") and not _QUOTED_DOTENV_TAIL_RE.fullmatch(t):
        msg = "Unexpected trailing characters after quoted value"
        raise ValueError(msg)


def _scan_double_quoted_dotenv_val(
    val: str,
) -> str:
    buf: list[str] = []
    idx = 1
    while idx < len(val):
        ch = val[idx]
        if ch == "\\":
            if idx + 1 >= len(val):
                break
            nxt = val[idx + 1]
            buf.append(_DOUBLE_QUOTE_ESCAPES.get(nxt, f"\\{nxt}"))
            idx += 2
            continue
        if ch == '"':
            _validate_quoted_dotenv_tail(val[idx + 1 :])
            return "".join(buf)
        buf.append(ch)
        idx += 1
    msg = "Unterminated double-quoted value"
    raise ValueError(msg)


def _load_dotenv(
    text: str, environment: Mapping[str, str], budget: InterpolationBudget
) -> dict[str, str]:
    env_vars: dict[str, str] = {}
    for raw_line in text.splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("export "):
            line = line[7:].strip()
        if "=" not in line:
            continue
        key, _, val = line.partition("=")
        key = key.strip()
        val = val.strip()
        if val.startswith("'"):
            close_idx = val.find("'", 1)
            if close_idx < 0:
                msg = "Unterminated single-quoted value"
                raise ValueError(msg)
            _validate_quoted_dotenv_tail(val[close_idx + 1 :])
            val = val[1:close_idx]
        elif val.startswith('"'):
            inner = _scan_double_quoted_dotenv_val(val)
            val = _interpolate_compose_str(inner, {**env_vars, **environment}, budget)
        else:
            val = _INLINE_COMMENT_RE.sub("", val).strip()
            val = _interpolate_compose_str(val, {**env_vars, **environment}, budget)
        if key:
            env_vars[key] = val
    return env_vars


def _eval_braced_compose_var(
    inner: str, env_lookup: dict[str, str], budget: InterpolationBudget, _depth: int
) -> str:
    m = _BRACED_VAR_HEAD_RE.match(inner)
    if m is None:
        msg = "Invalid Compose variable interpolation format"
        raise ValueError(msg)
    name = m.group(1)
    op = m.group(2)
    raw_arg = m.group(3) or ""
    if op is None:
        return env_lookup.get(name, "")
    if op == ":-":
        val = env_lookup.get(name, "")
        return (
            val if val != "" else _interpolate_compose_str(raw_arg, env_lookup, budget, _depth + 1)
        )
    if op == "-":
        return (
            env_lookup[name]
            if name in env_lookup
            else _interpolate_compose_str(raw_arg, env_lookup, budget, _depth + 1)
        )
    if op == ":+":
        val = env_lookup.get(name, "")
        return (
            _interpolate_compose_str(raw_arg, env_lookup, budget, _depth + 1) if val != "" else ""
        )
    if op == "+":
        return (
            _interpolate_compose_str(raw_arg, env_lookup, budget, _depth + 1)
            if name in env_lookup
            else ""
        )
    if op == ":?":
        val = env_lookup.get(name, "")
        if not val:
            arg = _interpolate_compose_str(raw_arg, env_lookup, budget, _depth + 1)
            msg = (
                arg if "$" not in raw_arg else ""
            ) or f"Required Compose variable {name!r} is unset or empty"
            raise ValueError(msg)
        return val
    if op == "?":
        if name not in env_lookup:
            arg = _interpolate_compose_str(raw_arg, env_lookup, budget, _depth + 1)
            msg = (
                arg if "$" not in raw_arg else ""
            ) or f"Required Compose variable {name!r} is unset"
            raise ValueError(msg)
        return env_lookup[name]
    return ""


def _interpolate_compose_str(
    text: str, env_lookup: dict[str, str], budget: InterpolationBudget, _depth: int = 0
) -> str:
    budget.depth(_depth)
    if "$" not in text:
        budget.consume(text)
        return text
    out = Fragments(budget)
    i = 0
    n = len(text)
    while i < n:
        ch = text[i]
        if ch != "$":
            out.append(ch)
            i += 1
            continue
        if i + 1 < n and text[i + 1] == "$":
            out.append("$")
            i += 2
            continue
        if i + 1 < n and text[i + 1] == "{":
            depth = 1
            j = i + 2
            while j < n and depth > 0:
                if text[j : j + 2] == "${":
                    depth += 1
                    j += 2
                elif text[j] == "}":
                    depth -= 1
                    j += 1
                else:
                    j += 1
            if depth == 0:
                out.append(
                    _eval_braced_compose_var(text[i + 2 : j - 1], env_lookup, budget, _depth)
                )
                i = j
                continue
            msg = "Invalid Compose variable interpolation (unclosed braced variable)"
            raise ValueError(msg)
        var_match = _BARE_VAR_RE.match(text, i + 1)
        if var_match is not None:
            out.append(env_lookup.get(var_match.group(1), ""))
            i = var_match.end()
            continue
        out.append("$")
        i += 1
    return "".join(out)
