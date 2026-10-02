"""Inspect-agnostic container specification for building and running containers in Capsem VMs."""

from __future__ import annotations

from dataclasses import dataclass, field, replace
from typing import Any

__all__ = [
    "ContainerSpec",
    "_copy_spec_with_updates",
]


@dataclass(frozen=True)
class ContainerSpec:
    """Inspect-agnostic specification for building and running a container inside a Capsem VM."""

    image: str = "python:3.11-slim"
    working_dir: str = "/workspace"
    dockerfile: str | None = None
    build_context: str | None = None
    build_args: dict[str, str] = field(default_factory=dict)
    build_target: str | None = None
    environment: dict[str, str] = field(default_factory=dict)
    command: tuple[str, ...] | str | None = None
    entrypoint: tuple[str, ...] | str | None = None
    volumes: tuple[str, ...] = ()
    ports: tuple[str, ...] = ()
    expose: tuple[str, ...] = ()
    healthcheck: dict[str, Any] | None = None
    init: bool = False
    mem_limit: str | None = None
    network_mode: str | None = None
    user: str | None = None
    build_timeout: int = 3600
    explicit_fields: frozenset[str] = field(default_factory=frozenset)


def _copy_spec_with_updates(spec: ContainerSpec, updates: dict[str, Any]) -> ContainerSpec:
    """Return a new `ContainerSpec` with `updates` applied and recorded in `explicit_fields`."""
    return replace(spec, **updates, explicit_fields=spec.explicit_fields | frozenset(updates))
