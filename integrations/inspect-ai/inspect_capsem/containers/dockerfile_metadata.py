"""Pure Dockerfile support from Pierre Tholoniat's bb61fc82d integration."""

from __future__ import annotations

import posixpath
import re
import shlex

from .dockerfile_ca import is_root_user_spec
from .dockerfile_scan import _dockerfile_instructions

_DOCKERFILE_VAR_RE = re.compile(r"\$\{([A-Za-z_]\w*)(?::(-|\+)([^}]*))?\}|\$([A-Za-z_]\w*)")


def _expand_dockerfile_vars(value: str, env_vars: dict[str, str]) -> str:
    def _repl(match: re.Match[str]) -> str:
        braced_name, op, default_val, bare_name = match.groups()
        name = braced_name or bare_name
        val = env_vars.get(name)
        if op == "-":
            return val if val else (default_val or "")
        if op == "+":
            return (default_val or "") if val else ""
        if val is not None:
            return val
        return match.group(0)

    return _DOCKERFILE_VAR_RE.sub(_repl, value)


def _strip_optional_quotes(value: str) -> str:
    val = value.strip()
    if len(val) >= 2 and val[0] == val[-1] and val[0] in ("'", '"'):
        return val[1:-1]
    return val


def _update_dockerfile_vars(
    keyword: str, joined: str, target_vars: dict[str, str], fallback_vars: dict[str, str]
) -> None:
    parts = joined.split(None, 1)
    if len(parts) < 2:
        return
    rest = parts[1].strip()
    if not rest:
        return
    first_token = rest.split(None, 1)[0]
    if keyword == "ENV" and "=" not in first_token:
        kv = rest.split(None, 1)
        if len(kv) == 2:
            target_vars[kv[0]] = _expand_dockerfile_vars(_strip_optional_quotes(kv[1]), target_vars)
        return
    try:
        tokens = shlex.split(rest)
    except ValueError:
        tokens = rest.split()
    for tok in tokens:
        if "=" in tok:
            k, _, v = tok.partition("=")
            if k:
                target_vars[k] = _expand_dockerfile_vars(_strip_optional_quotes(v), target_vars)
        elif keyword == "ARG" and tok in fallback_vars and tok not in target_vars:
            target_vars[tok] = fallback_vars[tok]


def _extract_dockerfile_metadata(text: str) -> tuple[str | None, str | None]:
    """Return `(workdir, user)` for the final stage of a Dockerfile (`None` when unset/root)."""
    instructions = _dockerfile_instructions(text.splitlines())
    global_vars: dict[str, str] = {}
    stage_workdirs: dict[str, str | None] = {}
    stage_users: dict[str, str | None] = {}
    stage_vars: dict[str, dict[str, str]] = {}
    in_stage = False
    current_alias: str | None = None
    current_workdir: str | None = None
    current_user: str | None = None
    current_vars: dict[str, str] = {}

    for keyword, joined, _ in instructions:
        if keyword == "FROM":
            args = [a for a in joined.split()[1:] if not a.startswith("--")]
            raw_base = _expand_dockerfile_vars(args[0], global_vars) if args else "scratch"
            base = raw_base.lower()
            alias = args[2].lower() if len(args) >= 3 and args[1].lower() == "as" else None
            in_stage = True
            current_alias = alias
            current_workdir = stage_workdirs.get(base)
            current_user = stage_users.get(base)
            current_vars = dict(global_vars)
            if base in stage_vars:
                current_vars.update(stage_vars[base])
            if alias:
                stage_workdirs[alias] = current_workdir
                stage_users[alias] = current_user
                stage_vars[alias] = current_vars
        elif keyword in ("ARG", "ENV"):
            if in_stage:
                _update_dockerfile_vars(keyword, joined, current_vars, global_vars)
                if current_alias:
                    stage_vars[current_alias] = current_vars
            elif keyword == "ARG":
                _update_dockerfile_vars(keyword, joined, global_vars, global_vars)
        elif keyword == "WORKDIR" and in_stage:
            parts = joined.split(None, 1)
            if len(parts) > 1:
                raw_wd = _strip_optional_quotes(parts[1])
                expanded = _expand_dockerfile_vars(raw_wd, current_vars).strip()
                if expanded and "$" not in expanded:
                    if posixpath.isabs(expanded):
                        current_workdir = posixpath.normpath(expanded)
                    else:
                        current_workdir = posixpath.normpath(
                            posixpath.join(current_workdir or "/", expanded)
                        )
                    if current_alias:
                        stage_workdirs[current_alias] = current_workdir
        elif keyword == "USER" and in_stage:
            parts = joined.split(None, 1)
            if len(parts) > 1:
                raw_user = _strip_optional_quotes(parts[1])
                expanded_user = _expand_dockerfile_vars(raw_user, current_vars).strip()
                if expanded_user and "$" not in expanded_user:
                    current_user = None if is_root_user_spec(expanded_user) else expanded_user
                    if current_alias:
                        stage_users[current_alias] = current_user
    return current_workdir, current_user
