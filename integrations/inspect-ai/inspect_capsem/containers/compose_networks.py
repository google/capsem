"""Compose helpers derived from Pierre Tholoniat's bb61fc82d44bc42c978c4acf5435a1975600c75e."""

from __future__ import annotations

import logging
from typing import Any

from .compose_values import _field

logger = logging.getLogger(__name__)


def _is_internal_network_def(net_def: Any) -> bool:
    return bool(net_def is not None and _field(net_def, "internal", False))


def _resolve_compose_network_mode(selected_svc: Any, compose_cfg: Any) -> str | None:
    """Resolve `network_mode` from a Compose service or `networks:` with `internal: true`.

    Preserve the proposed frontend's offline intent when every selected
    network is internal. The backend must enforce that intent through its
    reviewed native controls; this parser neither supplies an engine nor
    authorizes a network namespace or a host-network fallback.
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
