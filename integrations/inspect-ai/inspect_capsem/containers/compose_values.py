"""Compose helpers derived from Pierre Tholoniat's bb61fc82d44bc42c978c4acf5435a1975600c75e."""

from __future__ import annotations

import logging
import math
import posixpath
import re
import shlex
from collections.abc import Mapping
from pathlib import Path
from typing import Any

from pydantic import BaseModel

from .compose_inputs import EMPTY_INPUTS, ComposeInputs

logger = logging.getLogger(__name__)
_NAMED_VOL_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]*$")
_MEM_RE = re.compile(r"^\s*(\d+(?:\.\d+)?)\s*([bkmg]i?b?)?\s*$", re.IGNORECASE)
_MEM_FACTORS = {
    u: 1024**p
    for p, us in enumerate((" b", "k kb kib", "m mb mib", "g gb gib"))
    for u in us.split(" ")
}


def _field(obj: Any, key: str, default: Any = None) -> Any:
    if isinstance(obj, BaseModel):
        obj = obj.model_dump(exclude_none=True, by_alias=True)
    if not isinstance(obj, dict):
        return default
    if key in obj:
        return obj[key]
    alt = key.replace("_", "-") if "_" in key else key.replace("-", "_")
    return obj.get(alt, default)


def _is_bind_mount_source(src: str) -> bool:
    return src.startswith((".", "/", "~")) or "/" in src or "\\" in src


def _resolve_declared_volumes(raw_top_volumes: Any) -> frozenset[str]:
    if raw_top_volumes is None:
        return frozenset()
    if not isinstance(raw_top_volumes, Mapping):
        raise ValueError("Top-level Compose 'volumes' must be a mapping.")
    declared: set[str] = set()
    for vol_name, vol_cfg in raw_top_volumes.items():
        name_str = str(vol_name).strip()
        if not _NAMED_VOL_RE.match(name_str):
            raise ValueError(f"Invalid top-level Compose volume name {name_str!r}.")
        if isinstance(vol_cfg, Mapping):
            driver = str(vol_cfg.get("driver") or "local").strip()
            if vol_cfg.get("external") or driver != "local" or vol_cfg.get("driver_opts"):
                raise ValueError(
                    f"Top-level Compose volume {name_str!r} uses unsupported external or "
                    "non-local driver options in Capsem OCI-workload mode."
                )
        declared.add(name_str)
    return frozenset(declared)


def _normalize_environment(svc_env: Any, inputs: ComposeInputs = EMPTY_INPUTS) -> dict[str, str]:
    if isinstance(svc_env, dict):
        env_map: dict[str, str] = {}
        for k, v in svc_env.items():
            key = str(k)
            if v is None:
                env_map[key] = inputs.lookup_env(key)
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
                env_map[text] = inputs.lookup_env(text)
        return env_map
    return {}


def _normalize_volumes(
    svc_vols: Any, base_dir: Path | None, inputs: ComposeInputs = EMPTY_INPUTS
) -> tuple[str, ...]:
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
            src_str = str(src)
            if vtype == "volume" and _is_bind_mount_source(src_str):
                raise ValueError(
                    f"Named Compose volume source {src_str!r} must be a volume name, not a path"
                )
            spec = f"{src_str}:{target}{':ro' if ro else ''}"
            is_bind = vtype == "bind"
        else:
            spec = str(item)
            src_head, _, _ = spec.partition(":")
            is_bind = _is_bind_mount_source(src_head)
        if is_bind and base_dir is not None and ":" in spec:
            src_part, _, rest = spec.partition(":")
            if src_part and not posixpath.isabs(src_part) and not src_part.startswith("~"):
                candidate = base_dir / src_part
                if src_part.startswith(".") or "/" in src_part or inputs.exists(candidate):
                    spec = f"{inputs.resolve(candidate)}:{rest}"
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


def parse_memory_to_ram_gb(raw_mem: Any) -> int:
    """Convert Compose memory limit (`512m`, `2g`, integer bytes) up to integer GiB (`>=1`)."""
    if isinstance(raw_mem, (int, float)) and not isinstance(raw_mem, bool):
        if raw_mem <= 0:
            raise ValueError(f"Invalid Compose memory limit: {raw_mem!r}")
        return max(1, math.ceil(float(raw_mem) / (1024**3)))
    m = _MEM_RE.match(str(raw_mem).strip())
    if not m:
        raise ValueError(f"Invalid Compose memory limit: {raw_mem!r}")
    amount, unit = float(m.group(1)), (m.group(2) or "").lower()
    if amount <= 0 or unit not in _MEM_FACTORS:
        raise ValueError(f"Invalid Compose memory limit: {raw_mem!r}")
    return max(1, math.ceil((amount * _MEM_FACTORS[unit]) / (1024**3)))


def parse_cpus_to_cpu_count(raw_cpus: Any) -> int:
    """Convert Compose `cpus` (`"1.5"`, `2`) up to integer vCPU count (`>=1`)."""
    try:
        val = float(raw_cpus)
    except (TypeError, ValueError) as exc:
        raise ValueError(f"Invalid Compose cpus limit: {raw_cpus!r}") from exc
    if val <= 0 or math.isnan(val) or math.isinf(val):
        raise ValueError(f"Invalid Compose cpus limit: {raw_cpus!r}")
    return max(1, math.ceil(val))


def _looks_like_host_bind_source(src: str) -> bool:
    return (
        src in (".", "..")
        or src.startswith(("/", "./", "../", "~"))
        or "/" in src
        or "\\" in src
        or (len(src) >= 2 and src[1] == ":" and src[0].isalpha())
    )


def _combine_entrypoint_and_command(
    entrypoint: tuple[str, ...] | str | None, command: tuple[str, ...] | str | None
) -> tuple[str, ...] | str | None:
    if entrypoint is None:
        return command
    ep_parts = tuple(shlex.split(entrypoint)) if isinstance(entrypoint, str) else entrypoint
    if not ep_parts or command is None:
        return ep_parts if ep_parts else command
    cmd_parts = tuple(shlex.split(command)) if isinstance(command, str) else command
    return (*ep_parts, *cmd_parts)
