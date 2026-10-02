"""Inspect-specific config coercion and Compose/Dockerfile default resolution."""

from __future__ import annotations

from pathlib import Path
from typing import TYPE_CHECKING, Any

from pydantic import BaseModel

from inspect_capsem.config import CapsemSandboxConfig
from inspect_capsem.containers.compose import (
    _COMPOSE_PASSTHROUGH_FIELDS,
    _is_bind_mount_source,
    extract_compose_fields,
    parse_compose_yaml_file,
)
from inspect_capsem.containers.dockerfile import resolve_dockerfile_defaults

if TYPE_CHECKING:
    from inspect_ai.util import SandboxEnvironmentConfigType

__all__ = [
    "_is_bind_mount_source",
    "coerce_config",
    "extract_compose_fields",
    "parse_compose_yaml_file",
    "resolve_compose_file",
]


def _copy_config_with_updates(
    cfg: CapsemSandboxConfig, updates: dict[str, Any]
) -> CapsemSandboxConfig:
    """Return a new `CapsemSandboxConfig` whose `model_fields_set` reflects `cfg` + `updates`."""
    explicit = {k: getattr(cfg, k) for k in cfg.model_fields_set}
    explicit.update(updates)
    return CapsemSandboxConfig(**explicit)


def _apply_dockerfile_defaults(cfg: CapsemSandboxConfig) -> CapsemSandboxConfig:
    if not cfg.dockerfile:
        return cfg
    updates = {
        k: v
        for k, v in resolve_dockerfile_defaults(cfg.dockerfile).items()
        if k not in cfg.model_fields_set
    }
    return _copy_config_with_updates(cfg, updates) if updates else cfg


def _apply_extracted_compose_fields(
    cfg: CapsemSandboxConfig, extracted: dict[str, Any]
) -> CapsemSandboxConfig:
    updates: dict[str, Any] = {}
    for field_name in (
        "execution_mode",
        "image",
        "dockerfile",
        "build_context",
        "working_dir",
        *_COMPOSE_PASSTHROUGH_FIELDS,
    ):
        if field_name in extracted and field_name not in cfg.model_fields_set:
            updates[field_name] = extracted[field_name]
    return _copy_config_with_updates(cfg, updates) if updates else cfg


def resolve_compose_file(cfg: CapsemSandboxConfig) -> CapsemSandboxConfig:
    if not cfg.compose_file:
        return cfg
    compose_path = Path(cfg.compose_file)
    if not compose_path.is_file():
        msg = f"Compose file not found: {cfg.compose_file}"
        raise FileNotFoundError(msg)
    parsed = parse_compose_yaml_file(compose_path)
    extracted = extract_compose_fields(parsed, base_dir=compose_path.parent)
    return _apply_extracted_compose_fields(cfg, extracted)


def _is_compose_config_model(config: Any) -> bool:
    return (
        isinstance(config, BaseModel)
        and not isinstance(config, CapsemSandboxConfig)
        and hasattr(config, "services")
    )


def coerce_config(
    config: SandboxEnvironmentConfigType | dict[str, Any] | str | None,
    *,
    resolve_compose: bool = True,
) -> CapsemSandboxConfig:
    if config is None:
        return CapsemSandboxConfig()
    if isinstance(config, dict):
        config = CapsemSandboxConfig.model_validate(config)
    elif isinstance(config, str):
        path = Path(config)
        if path.name.startswith(("Dockerfile", "Containerfile")) or path.suffix in (
            ".dockerfile",
            ".containerfile",
        ):
            config = CapsemSandboxConfig(execution_mode="container", dockerfile=str(path))
        elif path.suffix in (".yaml", ".yml"):
            config = CapsemSandboxConfig(execution_mode="container", compose_file=str(path))
        else:
            return CapsemSandboxConfig(execution_mode="container", image=config)
    elif _is_compose_config_model(config):
        extracted = extract_compose_fields(config)
        config = CapsemSandboxConfig(**extracted)
    elif not isinstance(config, CapsemSandboxConfig):
        msg = f"Unsupported Capsem sandbox config type: {type(config).__name__}"
        raise TypeError(msg)

    if "execution_mode" not in config.model_fields_set and (
        config.dockerfile or config.compose_file
    ):
        config = _copy_config_with_updates(config, {"execution_mode": "container"})
    if resolve_compose and config.compose_file:
        return resolve_compose_file(config)
    return _apply_dockerfile_defaults(config)
