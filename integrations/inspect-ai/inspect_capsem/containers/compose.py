"""Docker Compose YAML parsing and service/network/port field extraction."""

from __future__ import annotations

import logging
import os
import posixpath
import re
from pathlib import Path
from typing import Any

import yaml
from pydantic import BaseModel

from inspect_capsem.containers.dockerfile import resolve_dockerfile_defaults

__all__ = [
    "_COMPOSE_PASSTHROUGH_FIELDS",
    "_extract_service_fields",
    "_is_bind_mount_source",
    "extract_compose_fields",
    "parse_compose_yaml_file",
]

logger = logging.getLogger(__name__)


def _is_bind_mount_source(src: str) -> bool:
    return src.startswith((".", "/", "~")) or "/" in src or "\\" in src


_COMPOSE_PASSTHROUGH_FIELDS = (
    "build_args",
    "build_target",
    "environment",
    "command",
    "entrypoint",
    "volumes",
    "ports",
    "expose",
    "healthcheck",
    "init",
    "mem_limit",
    "network_mode",
    "user",
)
_BRACED_VAR_HEAD_RE = re.compile(
    r"^([A-Za-z_][A-Za-z0-9_]*)(?:(:-|-|:\?|\?|:\+|\+)(.*))?$", re.DOTALL
)
_BARE_VAR_RE = re.compile(r"([A-Za-z_][A-Za-z0-9_]*)")
_INLINE_COMMENT_RE = re.compile(r" +#.*$")
_DOUBLE_QUOTE_ESCAPES = {'"': '"', "\\": "\\", "$": "$$", "n": "\n", "r": "\r", "t": "\t"}


def _field(obj: Any, key: str, default: Any = None) -> Any:
    if isinstance(obj, BaseModel):
        obj = obj.model_dump(exclude_none=True, by_alias=True)
    if not isinstance(obj, dict):
        return default
    if key in obj:
        return obj[key]
    alt = key.replace("_", "-") if "_" in key else key.replace("-", "_")
    return obj.get(alt, default)


_QUOTED_DOTENV_TAIL_RE = re.compile(r"^[A-Za-z0-9_.\-]+$")


def _validate_quoted_dotenv_tail(tail: str, dotenv_path: Path, raw_line: str) -> None:
    t = tail.strip()
    if t and not t.startswith("#") and not _QUOTED_DOTENV_TAIL_RE.fullmatch(t):
        msg = f"Unexpected trailing characters after quoted value in {dotenv_path}: {raw_line!r}"
        raise ValueError(msg)


def _scan_double_quoted_dotenv_val(val: str, dotenv_path: Path, raw_line: str) -> str:
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
            _validate_quoted_dotenv_tail(val[idx + 1 :], dotenv_path, raw_line)
            return "".join(buf)
        buf.append(ch)
        idx += 1
    msg = f"Unterminated double-quoted value in {dotenv_path}: {raw_line!r}"
    raise ValueError(msg)


def _load_dotenv(dotenv_path: Path) -> dict[str, str]:
    if not dotenv_path.is_file():
        return {}
    env_vars: dict[str, str] = {}
    for raw_line in dotenv_path.read_text(encoding="utf-8").splitlines():
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
                msg = f"Unterminated single-quoted value in {dotenv_path}: {raw_line!r}"
                raise ValueError(msg)
            _validate_quoted_dotenv_tail(val[close_idx + 1 :], dotenv_path, raw_line)
            val = val[1:close_idx]
        elif val.startswith('"'):
            inner = _scan_double_quoted_dotenv_val(val, dotenv_path, raw_line)
            val = _interpolate_compose_str(inner, {**env_vars, **os.environ})
        else:
            val = _INLINE_COMMENT_RE.sub("", val).strip()
            val = _interpolate_compose_str(val, {**env_vars, **os.environ})
        if key:
            env_vars[key] = val
    return env_vars


def _eval_braced_compose_var(inner: str, env_lookup: dict[str, str]) -> str:
    m = _BRACED_VAR_HEAD_RE.match(inner)
    if m is None:
        msg = f"Invalid Compose variable interpolation format: '${{{inner}}}'"
        raise ValueError(msg)
    name = m.group(1)
    op = m.group(2)
    raw_arg = m.group(3) or ""
    if op is None:
        return env_lookup.get(name, "")
    if op == ":-":
        val = env_lookup.get(name, "")
        return val if val != "" else _interpolate_compose_str(raw_arg, env_lookup)
    if op == "-":
        return (
            env_lookup[name]
            if name in env_lookup
            else _interpolate_compose_str(raw_arg, env_lookup)
        )
    if op == ":+":
        val = env_lookup.get(name, "")
        return _interpolate_compose_str(raw_arg, env_lookup) if val != "" else ""
    if op == "+":
        return _interpolate_compose_str(raw_arg, env_lookup) if name in env_lookup else ""
    if op == ":?":
        val = env_lookup.get(name, "")
        if not val:
            arg = _interpolate_compose_str(raw_arg, env_lookup)
            msg = arg or f"Required Compose variable {name!r} is unset or empty"
            raise ValueError(msg)
        return val
    if op == "?":
        if name not in env_lookup:
            arg = _interpolate_compose_str(raw_arg, env_lookup)
            msg = arg or f"Required Compose variable {name!r} is unset"
            raise ValueError(msg)
        return env_lookup[name]
    return ""


def _interpolate_compose_str(text: str, env_lookup: dict[str, str]) -> str:
    if "$" not in text:
        return text
    out: list[str] = []
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
                out.append(_eval_braced_compose_var(text[i + 2 : j - 1], env_lookup))
                i = j
                continue
            msg = f"Invalid Compose variable interpolation (unclosed '${{'): {text!r}"
            raise ValueError(msg)
        var_match = _BARE_VAR_RE.match(text, i + 1)
        if var_match is not None:
            out.append(env_lookup.get(var_match.group(1), ""))
            i = var_match.end()
            continue
        out.append("$")
        i += 1
    return "".join(out)


def _interpolate_compose_tree(node: Any, env_lookup: dict[str, str]) -> Any:
    if isinstance(node, str):
        return _interpolate_compose_str(node, env_lookup)
    if isinstance(node, dict):
        return {k: _interpolate_compose_tree(v, env_lookup) for k, v in node.items()}
    if isinstance(node, list):
        return [_interpolate_compose_tree(item, env_lookup) for item in node]
    if isinstance(node, tuple):
        return tuple(_interpolate_compose_tree(item, env_lookup) for item in node)
    return node


def parse_compose_yaml_file(compose_path: Path) -> dict[str, Any]:
    """Parse a Compose YAML file directly with PyYAML and interpolate Compose variables."""
    raw = yaml.safe_load(compose_path.read_text(encoding="utf-8"))
    if not isinstance(raw, dict):
        return {}
    dotenv_vars = _load_dotenv(compose_path.parent / ".env")
    env_lookup = {**dotenv_vars, **os.environ}
    return _interpolate_compose_tree(raw, env_lookup)


def _normalize_environment(svc_env: Any) -> dict[str, str]:
    if isinstance(svc_env, dict):
        env_map: dict[str, str] = {}
        for k, v in svc_env.items():
            key = str(k)
            if v is None:
                env_map[key] = os.environ.get(key, "")
            elif isinstance(v, bool):
                env_map[key] = "true" if v else "false"
            else:
                env_map[key] = str(v)
        return env_map
    if isinstance(svc_env, list | tuple):
        env_map = {}
        for item in svc_env:
            text = str(item)
            if "=" in text:
                k, _, v = text.partition("=")
                env_map[k] = v
            elif text:
                env_map[text] = os.environ.get(text, "")
        return env_map
    return {}


def _normalize_volumes(svc_vols: Any, base_dir: Path | None) -> tuple[str, ...]:
    resolved: list[str] = []
    for item in svc_vols:
        if isinstance(item, dict):
            vtype = str(item.get("type") or "bind")
            if vtype not in ("bind", "volume"):
                msg = (
                    f"Unsupported Compose volume type {vtype!r}; "
                    f"only 'bind' and 'volume' are supported"
                )
                raise ValueError(msg)
            src = item.get("source") or item.get("src")
            target = item.get("target") or item.get("destination") or item.get("dst")
            if not src or not target:
                msg = f"Compose volume dict requires both 'source' and 'target': {item!r}"
                raise ValueError(msg)
            extra_vol_keys = {str(k) for k in item} - {
                "type",
                "source",
                "src",
                "target",
                "destination",
                "dst",
                "read_only",
            }
            if extra_vol_keys:
                logger.warning(
                    "Ignoring unsupported Compose volume option(s) %s on %r",
                    sorted(extra_vol_keys),
                    item,
                )
            ro = bool(item.get("read_only", False))
            spec = f"{src}:{target}{':ro' if ro else ''}"
            is_bind = vtype == "bind"
        else:
            spec = str(item)
            src_head, _, _ = spec.partition(":")
            is_bind = _is_bind_mount_source(src_head)
        if is_bind and base_dir is not None and ":" in spec:
            src_part, _, rest = spec.partition(":")
            if src_part and not posixpath.isabs(src_part):
                candidate = base_dir / src_part
                if src_part.startswith(".") or "/" in src_part or candidate.exists():
                    spec = f"{candidate.resolve()}:{rest}"
        resolved.append(spec)
    return tuple(resolved)


def _normalize_ports(svc_ports: Any, field_name: str = "ports") -> tuple[str, ...]:
    normalized: list[str] = []
    for item in svc_ports:
        if isinstance(item, dict):
            target = item.get("target")
            if target is None or str(target) == "":
                msg = f"Compose {field_name} dict entry requires 'target': {item!r}"
                raise ValueError(msg)
            published = item.get("published")
            host_ip = item.get("host_ip")
            protocol = str(item.get("protocol") or "").strip().lower()
            if published is not None and str(published) != "":
                spec = f"{host_ip}:{published}:{target}" if host_ip else f"{published}:{target}"
            else:
                spec = str(target)
            if protocol and protocol != "tcp":
                spec = f"{spec}/{protocol}"
            normalized.append(spec)
        else:
            normalized.append(str(item))
    return tuple(normalized)


def _normalize_healthcheck(svc_hc: Any) -> dict[str, Any] | None:
    hc: dict[str, Any] = {}
    for key in (
        "test",
        "interval",
        "timeout",
        "start_period",
        "start_interval",
        "retries",
        "disable",
    ):
        val = _field(svc_hc, key)
        if val is not None:
            hc[key] = list(val) if isinstance(val, tuple) else val
    return hc or None


def _is_internal_network_def(net_def: Any) -> bool:
    return bool(net_def is not None and _field(net_def, "internal", False))


def _resolve_compose_network_mode(selected_svc: Any, compose_cfg: Any) -> str | None:
    """Resolve `network_mode` from a Compose service or `networks:` with `internal: true`.

    In Capsem VMs, `dockerd` runs with `--bridge=none`, so custom Compose bridge networks
    cannot be created and containers default to `--network host`. However, if a service is
    attached *exclusively* to Compose networks marked `internal: true` (or has no explicit
    `networks:` while the top-level `default` network is `internal: true`), running it on
    `--network host` would violate network isolation and expose the internet. Mapping those
    services to `network_mode="none"` preserves offline isolation.
    """
    explicit = _field(selected_svc, "network_mode")
    if explicit:
        return str(explicit)
    compose_nets = _field(compose_cfg, "networks") if compose_cfg is not None else None
    svc_nets = _field(selected_svc, "networks")
    if isinstance(svc_nets, list | tuple) and svc_nets:
        net_names = [str(n) for n in svc_nets if n]
        if (
            net_names
            and isinstance(compose_nets, dict)
            and all(_is_internal_network_def(compose_nets.get(n)) for n in net_names)
        ):
            return "none"
        return None
    if isinstance(svc_nets, dict) and svc_nets:
        net_names = [str(k) for k in svc_nets if k]
        if net_names and all(
            _is_internal_network_def(svc_nets.get(n))
            or (isinstance(compose_nets, dict) and _is_internal_network_def(compose_nets.get(n)))
            for n in net_names
        ):
            return "none"
        return None
    if isinstance(compose_nets, dict) and _is_internal_network_def(compose_nets.get("default")):
        return "none"
    return None


def _parse_port_spec(spec: str) -> tuple[str | None, str]:
    """Return `(host_port_or_none, container_port)` for a Compose `ports` / `expose` item."""
    base = spec.strip().split("/", 1)[0]
    parts = base.split(":")
    if len(parts) == 1:
        return None, parts[0]
    return parts[-2], parts[-1]


def _warn_host_network_port_remapping(svc_name: str, fields: dict[str, Any]) -> None:
    """Log a warning when a host-networked Compose service remaps a host port."""
    net_mode = fields.get("network_mode") or "host"
    if net_mode not in ("host", "bridge", "default"):
        return
    for p_spec in fields.get("ports", ()):
        host_port, container_port = _parse_port_spec(str(p_spec))
        if host_port and container_port and host_port != container_port:
            logger.warning(
                "Compose service %r maps port %r under --network host; Docker ignores port "
                "remapping on host networking (container listens on port %s)",
                svc_name,
                p_spec,
                container_port,
            )


def _reject_k8s_allow_domains(selected_svc: Any, compose_cfg: Any = None) -> None:
    for scope in (selected_svc, compose_cfg):
        if scope is None:
            continue
        k8s_ext = _field(scope, "x-inspect_k8s_sandbox") or _field(scope, "x_inspect_k8s_sandbox")
        domains = _field(k8s_ext, "allow_domains") if k8s_ext else _field(scope, "allow_domains")
        if domains:
            msg = (
                "x-inspect_k8s_sandbox.allow_domains is not supported: Capsem enforces "
                "domain policy at the VM profile level, not per-container/per-sample"
            )
            raise NotImplementedError(msg)


_SUPPORTED_COMPOSE_SERVICE_KEYS = frozenset(
    {
        "allow_domains",
        "build",
        "command",
        "depends_on",
        "deploy",
        "entrypoint",
        "env_file",
        "environment",
        "expose",
        "healthcheck",
        "image",
        "init",
        "mem_limit",
        "network_mode",
        "networks",
        "ports",
        "user",
        "volumes",
        "working_dir",
        "x_inspect_k8s_sandbox",
    }
)


def _extract_service_fields(
    selected_svc: Any,
    compose_cfg: Any = None,
    base_dir: Path | None = None,
    *,
    service_name: str = "default",
) -> dict[str, Any]:
    _reject_k8s_allow_domains(selected_svc, compose_cfg)
    if _field(selected_svc, "depends_on") is not None:
        msg = (
            "Compose 'depends_on' is not supported by inspect-capsem "
            "(only single-service Compose files are supported)"
        )
        raise ValueError(msg)
    if _field(selected_svc, "env_file") is not None:
        msg = (
            "Compose service 'env_file' is not supported by inspect-capsem; "
            "use 'environment' or a project .env file"
        )
        raise ValueError(msg)
    if isinstance(selected_svc, dict):
        unknown_svc_keys = sorted(
            str(k)
            for k in selected_svc
            if str(k) not in _SUPPORTED_COMPOSE_SERVICE_KEYS and not str(k).startswith("x-")
        )
        if unknown_svc_keys:
            logger.warning(
                "Ignoring unsupported compose keys in service %r: %s",
                service_name,
                ", ".join(unknown_svc_keys),
            )
    overrides: dict[str, Any] = {"execution_mode": "container"}
    svc_image = _field(selected_svc, "image")
    if svc_image:
        overrides["image"] = str(svc_image)
    svc_build = _field(selected_svc, "build")
    if svc_build:
        if isinstance(svc_build, str):
            build_path = (base_dir / svc_build) if base_dir else Path(svc_build)
            df_path = (build_path / "Dockerfile") if build_path.is_dir() else build_path
            overrides["dockerfile"] = str(df_path)
        else:
            if _field(svc_build, "dockerfile_inline") is not None:
                msg = "Compose 'build.dockerfile_inline' is not supported by inspect-capsem"
                raise ValueError(msg)
            if isinstance(svc_build, dict):
                unknown_build_keys = {str(k) for k in svc_build} - {
                    "context",
                    "dockerfile",
                    "args",
                    "target",
                }
                if unknown_build_keys:
                    msg = f"Unsupported Compose build option(s): {sorted(unknown_build_keys)}"
                    raise ValueError(msg)
            ctx = _field(svc_build, "context") or "."
            df_name = _field(svc_build, "dockerfile") or "Dockerfile"
            ctx_path = (base_dir / ctx) if base_dir else Path(ctx)
            df_path = ctx_path / df_name
            overrides["dockerfile"] = str(df_path)
            if df_path.parent != ctx_path:
                overrides["build_context"] = str(ctx_path)
            build_args = _normalize_environment(_field(svc_build, "args"))
            if build_args:
                overrides["build_args"] = build_args
            build_target = _field(svc_build, "target")
            if build_target:
                overrides["build_target"] = str(build_target)
    svc_workdir = _field(selected_svc, "working_dir")
    if svc_workdir:
        overrides["working_dir"] = str(svc_workdir)
    svc_env = _normalize_environment(_field(selected_svc, "environment"))
    if svc_env:
        overrides["environment"] = svc_env
    for cmd_key in ("command", "entrypoint"):
        val = _field(selected_svc, cmd_key)
        if val is not None:
            overrides[cmd_key] = (
                tuple(str(x) for x in val) if isinstance(val, list | tuple) else str(val)
            )
    svc_vols = _field(selected_svc, "volumes")
    if isinstance(svc_vols, list | tuple) and svc_vols:
        overrides["volumes"] = _normalize_volumes(svc_vols, base_dir)
    for port_key in ("ports", "expose"):
        val = _field(selected_svc, port_key)
        if isinstance(val, list | tuple) and val:
            overrides[port_key] = _normalize_ports(val, field_name=port_key)
    svc_hc = _field(selected_svc, "healthcheck")
    if svc_hc:
        norm_hc = _normalize_healthcheck(svc_hc)
        if norm_hc:
            overrides["healthcheck"] = norm_hc
    svc_init = _field(selected_svc, "init")
    if svc_init is not None:
        overrides["init"] = bool(svc_init)
    svc_mem = _field(selected_svc, "mem_limit")
    if not svc_mem:
        deploy = _field(selected_svc, "deploy")
        resources = _field(deploy, "resources") if deploy else None
        limits = _field(resources, "limits") if resources else None
        svc_mem = _field(limits, "memory") if limits else None
    if svc_mem:
        overrides["mem_limit"] = str(svc_mem)
    net_mode = _resolve_compose_network_mode(selected_svc, compose_cfg)
    if net_mode:
        overrides["network_mode"] = net_mode
    svc_user = _field(selected_svc, "user")
    if svc_user:
        overrides["user"] = str(svc_user)
    if "dockerfile" in overrides:
        for k, v in resolve_dockerfile_defaults(overrides["dockerfile"]).items():
            overrides.setdefault(k, v)
    return overrides


def extract_compose_fields(compose_cfg: Any, base_dir: Path | None = None) -> dict[str, Any]:
    """Extract `CapsemSandboxConfig` overrides from a single-service Compose config."""
    services = _field(compose_cfg, "services")
    if not isinstance(services, dict) or not services:
        raise ValueError(
            "CapsemComposeError: compose file must define a non-empty 'services' mapping"
        )
    if len(services) > 1:
        names = ", ".join(str(k) for k in services)
        msg = (
            f"Multi-service Compose files are not supported by inspect-capsem; "
            f"found {len(services)} services: {names}"
        )
        raise ValueError(msg)
    svc_name, selected_svc = next(iter(services.items()))
    fields = _extract_service_fields(
        selected_svc, compose_cfg, base_dir=base_dir, service_name=str(svc_name)
    )
    _warn_host_network_port_remapping(str(svc_name), fields)
    return fields
