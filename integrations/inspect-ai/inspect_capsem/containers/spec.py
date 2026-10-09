"""Inspect-free OCI container specification value object."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


@dataclass(frozen=True)
class ContainerSpec:
    """Self-contained OCI container launch specification."""

    image: str | None = None
    working_dir: str = "/workspace"
    working_dir_explicit: bool = False
    environment: dict[str, str] = field(default_factory=dict)
    command: tuple[str, ...] | str | None = None
    volumes: tuple[str, ...] = ()
    healthcheck: dict[str, Any] | None = None
    mem_limit: str | None = None
    user: str | None = None
    allowed_host_paths: tuple[str, ...] = ()
