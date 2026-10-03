"""Configuration model for the Capsem Inspect AI SandboxEnvironment."""

from __future__ import annotations

from typing import Any, Literal

from pydantic import BaseModel, Field, model_validator

from inspect_capsem.containers.spec import ContainerSpec


class CapsemSandboxConfig(BaseModel, frozen=True, extra="forbid"):
    """Configuration for `CapsemSandboxEnvironment`.

    Supports both VM-level execution (`execution_mode="vm"`, running directly
    inside a Capsem VM or `capsem.Hypervisor` session) and nested container
    execution (`execution_mode="container"`, running `docker exec` inside a
    Capsem VM).
    """

    @model_validator(mode="before")
    @classmethod
    def _reject_unsupported_knobs(cls, data: Any) -> Any:
        if isinstance(data, dict):
            if data.get("allow_domains") is not None:
                msg = (
                    "CapsemSandboxConfig does not support per-VM allow_domains; "
                    "configure domain policy in the Capsem profile"
                )
                raise NotImplementedError(msg)
            if data.get("host_workspace_dir") is not None:
                msg = (
                    "host_workspace_dir is not supported: Capsem does not expose "
                    "a host VirtioFS mount"
                )
                raise NotImplementedError(msg)
        return data

    execution_mode: Literal["vm", "container"] = Field(default="vm")
    template: str = Field(default="code")
    image: str = Field(default="python:3.11-slim")
    cpu_count: int = Field(default=4)
    ram_gb: int = Field(default=8)
    working_dir: str = Field(default="/workspace")
    dockerfile: str | None = Field(default=None)
    build_context: str | None = Field(default=None)
    build_args: dict[str, str] = Field(default_factory=dict)
    build_target: str | None = Field(default=None)
    compose_file: str | None = Field(default=None)
    environment: dict[str, str] = Field(default_factory=dict)
    command: tuple[str, ...] | str | None = Field(default=None)
    entrypoint: tuple[str, ...] | str | None = Field(default=None)
    volumes: tuple[str, ...] = Field(default_factory=tuple)
    ports: tuple[str, ...] = Field(default_factory=tuple)
    expose: tuple[str, ...] = Field(default_factory=tuple)
    healthcheck: dict[str, Any] | None = None
    init: bool = Field(default=False)
    mem_limit: str | None = Field(default=None)
    network_mode: str | None = Field(default=None)
    user: str | None = Field(default=None)
    build_timeout: int = Field(default=3600)

    def to_container_spec(self) -> ContainerSpec:
        """Convert this `CapsemSandboxConfig` into an Inspect-agnostic `ContainerSpec`."""
        return ContainerSpec(
            image=self.image,
            working_dir=self.working_dir,
            dockerfile=self.dockerfile,
            build_context=self.build_context,
            build_args=dict(self.build_args),
            build_target=self.build_target,
            environment=dict(self.environment),
            command=self.command,
            entrypoint=self.entrypoint,
            volumes=self.volumes,
            ports=self.ports,
            expose=self.expose,
            healthcheck=dict(self.healthcheck) if self.healthcheck is not None else None,
            init=self.init,
            mem_limit=self.mem_limit,
            network_mode=self.network_mode,
            user=self.user,
            build_timeout=self.build_timeout,
            explicit_fields=frozenset(self.model_fields_set),
        )

    def __hash__(self) -> int:
        return hash(self.model_dump_json())
