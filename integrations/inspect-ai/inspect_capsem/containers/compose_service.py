"""Compose helpers derived from Pierre Tholoniat's bb61fc82d44bc42c978c4acf5435a1975600c75e."""

from __future__ import annotations

import logging
from pathlib import Path
from typing import Any

from .compose_inputs import EMPTY_INPUTS, ComposeInputs
from .compose_networks import (
    _reject_k8s_allow_domains,
    _resolve_compose_network_mode,
    _warn_host_network_port_remapping,
)
from .compose_values import (
    _field,
    _normalize_environment,
    _normalize_healthcheck,
    _normalize_ports,
    _normalize_volumes,
)

logger = logging.getLogger(__name__)


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


def _extract_service_fields(
    selected_svc: Any,
    compose_cfg: Any = None,
    base_dir: Path | None = None,
    *,
    service_name: str = "default",
    inputs: ComposeInputs = EMPTY_INPUTS,
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
            df_path = (build_path / "Dockerfile") if inputs.is_dir(build_path) else build_path
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
            build_args = _normalize_environment(_field(svc_build, "args"), inputs)
            if build_args:
                overrides["build_args"] = build_args
            build_target = _field(svc_build, "target")
            if build_target:
                overrides["build_target"] = str(build_target)
    svc_workdir = _field(selected_svc, "working_dir")
    if svc_workdir:
        overrides["working_dir"] = str(svc_workdir)
    svc_env = _normalize_environment(_field(selected_svc, "environment"), inputs)
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
        overrides["volumes"] = _normalize_volumes(svc_vols, base_dir, inputs)
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
        for k, v in inputs.dockerfile_defaults(overrides["dockerfile"]).items():
            overrides.setdefault(k, v)
    return overrides


def extract_compose_fields(
    compose_cfg: Any, base_dir: Path | None = None, *, inputs: ComposeInputs = EMPTY_INPUTS
) -> dict[str, Any]:
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
        selected_svc, compose_cfg, base_dir=base_dir, service_name=str(svc_name), inputs=inputs
    )
    _warn_host_network_port_remapping(str(svc_name), fields)
    return fields
