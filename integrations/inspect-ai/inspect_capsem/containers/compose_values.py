"""Compose helpers derived from Pierre Tholoniat's bb61fc82d44bc42c978c4acf5435a1975600c75e."""

from __future__ import annotations

import logging
import posixpath
from pathlib import Path
from typing import Any

from pydantic import BaseModel

from .compose_inputs import EMPTY_INPUTS, ComposeInputs

logger = logging.getLogger(__name__)


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


def _normalize_environment(svc_env: Any, inputs: ComposeInputs = EMPTY_INPUTS) -> dict[str, str]:
    if isinstance(svc_env, dict):
        env_map: dict[str, str] = {}
        for k, v in svc_env.items():
            key = str(k)
            if v is None:
                env_map[key] = inputs.environment.get(key, "")
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
                env_map[text] = inputs.environment.get(text, "")
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
