"""Evaluator-granted host image build policy (`HostBuildGrant` and env-authority narrowing)."""

from __future__ import annotations

import os
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Literal

from .build_context import (
    _is_within,
    prepare_compose_build_service,
    resolve_context_and_dockerfile,
)
from .compose_fields import CAPSEM_INSPECT_ALLOWED_HOST_PATHS_VAR, _realpath

_TRUE_VALUES = frozenset({"1", "true", "yes", "on"})
_FALSE_VALUES = frozenset({"0", "false", "no", "off"})
_ALLOWED_BUILD_NETWORKS = frozenset({"none", "default"})
_GRANT_FIELDS = frozenset({"enabled", "allowed_contexts", "network"})
_RESOLVED_BUILD_KEYS = frozenset(
    {"context", "dockerfile", "args", "target", "network", "docker_config_dir"}
)


@dataclass(frozen=True)
class HostBuildGrant:
    """Task-level host-build configuration narrowing the evaluator environment."""

    enabled: bool = True
    allowed_contexts: tuple[str, ...] = ()
    network: Literal["none", "default"] | None = None

    def __post_init__(self) -> None:
        clean_ctx = tuple(str(p).strip() for p in self.allowed_contexts if str(p).strip())
        object.__setattr__(self, "allowed_contexts", clean_ctx)
        if self.network == "host":
            raise ValueError("Host build policy forbids network='host'; allowed: 'none', 'default'")
        if self.network is not None and self.network not in _ALLOWED_BUILD_NETWORKS:
            raise ValueError(f"Unsupported host build network {self.network!r}")

    @classmethod
    def from_value(cls, value: Any) -> HostBuildGrant | None:
        if value is None or isinstance(value, HostBuildGrant):
            return value
        if isinstance(value, bool):
            return cls(enabled=value)
        if isinstance(value, Mapping):
            if unknown := sorted({str(k) for k in value} - _GRANT_FIELDS):
                raise ValueError(f"Unknown HostBuildGrant fields: {unknown}")
            raw_ctx = value.get("allowed_contexts") or ()
            items = raw_ctx.split(",") if isinstance(raw_ctx, str) else raw_ctx
            raw_net = value.get("network")
            net: Any = str(raw_net).strip().lower() if raw_net is not None else None
            return cls(
                enabled=bool(value.get("enabled", True)),
                allowed_contexts=tuple(str(p).strip() for p in items if str(p).strip()),
                network=net,
            )
        raise TypeError(f"Expected HostBuildGrant, mapping, or bool; got {type(value)!r}")

    def to_dict(self) -> dict[str, Any]:
        return {
            "enabled": self.enabled,
            "allowed_contexts": list(self.allowed_contexts),
            "network": self.network,
        }


def is_host_build_enabled(host_build: Any = None) -> bool:
    """Return True only when `CAPSEM_INSPECT_HOST_BUILD=1` in env and not disabled by task."""
    env_raw = os.environ.get("CAPSEM_INSPECT_HOST_BUILD", "").strip().lower()
    if env_raw and env_raw not in (_TRUE_VALUES | _FALSE_VALUES):
        raise ValueError(
            f"Invalid CAPSEM_INSPECT_HOST_BUILD={env_raw!r}; expected 1/true or 0/false"
        )
    if env_raw not in _TRUE_VALUES:
        return False
    grant = HostBuildGrant.from_value(host_build)
    return not (grant is not None and not grant.enabled)


def _resolve_build_network(grant: HostBuildGrant | None) -> str:
    env_net = os.environ.get("CAPSEM_INSPECT_BUILD_NETWORK", "").strip().lower() or "none"
    if env_net == "host":
        raise ValueError("Host build policy forbids network='host' in CAPSEM_INSPECT_BUILD_NETWORK")
    if env_net not in _ALLOWED_BUILD_NETWORKS:
        raise ValueError(f"Unsupported CAPSEM_INSPECT_BUILD_NETWORK={env_net!r}")
    if grant is not None and grant.network is not None:
        if grant.network == "default" and env_net == "none":
            raise ValueError(
                "HostBuildGrant(network='default') exceeds operator ceiling "
                "CAPSEM_INSPECT_BUILD_NETWORK='none'"
            )
        return grant.network
    return env_net


def _resolve_docker_config_dir(operator_roots: tuple[Path, ...]) -> str | None:
    env_dcfg = os.environ.get("CAPSEM_INSPECT_BUILD_DOCKER_CONFIG", "").strip()
    if not env_dcfg:
        return None
    env_real = _realpath(env_dcfg)
    if not env_real.is_dir():
        raise ValueError(f"CAPSEM_INSPECT_BUILD_DOCKER_CONFIG directory not found: {env_dcfg}")
    if not any(_is_within(env_real, r) for r in operator_roots):
        raise ValueError(
            f"CAPSEM_INSPECT_BUILD_DOCKER_CONFIG {str(env_real)!r} is not within "
            "CAPSEM_INSPECT_ALLOWED_HOST_PATHS"
        )
    return str(env_real)


def _narrow_roots(
    candidates: Sequence[str], ceiling: tuple[Path, ...], label: str
) -> tuple[Path, ...]:
    raw_items = tuple(r.strip() for r in candidates if r and r.strip())
    if not raw_items:
        return ceiling
    if not ceiling:
        raise ValueError(f"{label} cannot widen unset CAPSEM_INSPECT_ALLOWED_HOST_PATHS")
    narrowed: list[Path] = []
    for raw in raw_items:
        p = _realpath(raw)
        if not any(_is_within(p, c) for c in ceiling):
            raise ValueError(
                f"{label} path {str(p)!r} is not allowlisted in CAPSEM_INSPECT_ALLOWED_HOST_PATHS"
            )
        narrowed.append(p)
    return tuple(narrowed)


def _resolve_allowed_build_roots(
    allowed_host_paths: Sequence[str], grant: HostBuildGrant | None
) -> tuple[tuple[Path, ...], tuple[Path, ...]]:
    raw_op = os.environ.get(CAPSEM_INSPECT_ALLOWED_HOST_PATHS_VAR, "")
    op_roots = tuple(
        _realpath(s.strip()) for c in raw_op.split(",") for s in c.split(os.pathsep) if s.strip()
    )
    eff_roots = _narrow_roots(allowed_host_paths, op_roots, "allowed_host_paths")
    if grant and grant.allowed_contexts:
        eff_roots = _narrow_roots(
            grant.allowed_contexts, eff_roots, "HostBuildGrant.allowed_contexts"
        )
    return op_roots, eff_roots


def resolve_effective_host_build(
    dockerfile: str,
    build_context: str | None = None,
    *,
    build_args: Mapping[str, str] | None = None,
    build_target: str | None = None,
    stanza: str = "build",
    allowed_host_paths: Sequence[str] = (),
    host_build: Any = None,
) -> dict[str, Any]:
    """Validate evaluator host-build grant, path containment, and return normalized build spec."""
    grant = HostBuildGrant.from_value(host_build)
    if not is_host_build_enabled(grant):
        raise ValueError(
            f"Host-side image build for {stanza!r} is disabled by default; "
            "set CAPSEM_INSPECT_HOST_BUILD=1 in the evaluator environment."
        )
    operator_roots, allowed_roots = _resolve_allowed_build_roots(allowed_host_paths, grant)
    if not allowed_roots:
        raise ValueError(
            f"Host-side image build for {stanza!r} requires CAPSEM_INSPECT_ALLOWED_HOST_PATHS "
            "(and optional allowed_host_paths within it)."
        )
    ctx_real, df_real = resolve_context_and_dockerfile(
        dockerfile, build_context, allowed_roots, stanza=stanza
    )
    return {
        "context": str(ctx_real),
        "dockerfile": str(df_real),
        "args": {str(k): str(v) for k, v in (build_args or {}).items()},
        "target": build_target,
        "network": _resolve_build_network(grant),
        "docker_config_dir": _resolve_docker_config_dir(operator_roots),
    }


def _is_resolved_build_spec(val: Mapping[str, Any]) -> bool:
    return (
        set(val.keys()) == _RESOLVED_BUILD_KEYS
        and isinstance(val.get("context"), str)
        and isinstance(val.get("dockerfile"), str)
        and isinstance(val.get("args"), Mapping)
        and val.get("network") in _ALLOWED_BUILD_NETWORKS
        and all(
            val.get(k) is None or isinstance(val.get(k), str)
            for k in ("target", "docker_config_dir")
        )
    )


def resolve_direct_host_build(
    svc: Mapping[str, Any],
    *,
    allowed_host_paths: Sequence[str] = (),
    allowed_host_env: Sequence[str] = (),
    host_build: Any = None,
) -> dict[str, Any] | None:
    """Parse and validate direct `CapsemSandboxConfig(build=..., dockerfile=...)` via Compose service parser."""
    raw_build = svc.get("build")
    if isinstance(raw_build, Mapping) and _is_resolved_build_spec(raw_build):
        return resolve_effective_host_build(
            str(raw_build["dockerfile"]),
            str(raw_build["context"]),
            build_args=raw_build["args"],
            build_target=raw_build.get("target"),
            allowed_host_paths=allowed_host_paths,
            host_build=host_build,
        )
    svc_clean, stanza = prepare_compose_build_service(svc, direct_config=True)
    if stanza is None:
        return None
    from .compose_inputs import build_host_compose_inputs
    from .compose_service import extract_compose_fields

    extracted = extract_compose_fields(
        {"services": {"default": svc_clean}},
        base_dir=None,
        inputs=build_host_compose_inputs(allowed_host_env=allowed_host_env),
    )
    return resolve_effective_host_build(
        extracted["dockerfile"],
        extracted.get("build_context"),
        build_args=extracted.get("build_args"),
        build_target=extracted.get("build_target"),
        stanza=stanza,
        allowed_host_paths=allowed_host_paths,
        host_build=host_build,
    )
