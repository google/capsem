"""Configuration model for the Capsem Inspect AI SandboxEnvironment."""

from __future__ import annotations

import logging
from collections.abc import Mapping
from pathlib import Path
from typing import TYPE_CHECKING, Any, Literal

from pydantic import BaseModel, Field, model_validator

if TYPE_CHECKING:
    from .containers import ContainerSpec, HostBuildGrant
else:
    HostBuildGrant = Any

logger = logging.getLogger(__name__)
_REJECTED_DOCKERFILE_KEYS = frozenset(
    {"build_context", "build_args", "build_target", "dockerfile_inline"}
)


def _json_hashable(value: Any) -> Any:
    if isinstance(value, dict):
        return tuple(sorted((str(k), _json_hashable(v)) for k, v in value.items()))
    if isinstance(value, (list, tuple)):
        return tuple(_json_hashable(item) for item in value)
    return value


class CapsemSandboxConfig(BaseModel, frozen=True, extra="forbid"):
    """Configuration for `CapsemSandboxEnvironment` (VM and container execution)."""

    @model_validator(mode="before")
    @classmethod
    def _reject_unsupported_knobs(cls, data: Any) -> Any:
        if isinstance(data, Mapping):
            if data.get("template") is not None:
                msg = (
                    "CapsemSandboxConfig does not support template selection yet; "
                    "Hypervisor.create uses the service default VM profile"
                )
                raise NotImplementedError(msg)
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
            if data.get("ports") not in (None, (), []):
                raise ValueError(
                    "Config field 'ports' is not supported in Capsem OCI-workload mode; "
                    "Capsem isolates workloads inside a micro-VM without host port forwarding."
                )
            for bad_key in _REJECTED_DOCKERFILE_KEYS:
                if data.get(bad_key) is not None:
                    raise ValueError(
                        f"Config field {bad_key!r} is not supported on CapsemSandboxConfig; "
                        "pass 'build' or 'dockerfile' or 'compose_file' instead."
                    )
            out = dict(data)
            out.pop("ports", None)
            out.pop("expose", None)
            if out.pop("init", None):
                logger.warning(
                    "Ignoring 'init=True' on CapsemSandboxConfig: Capsem OCI-workload mode "
                    "supervises the container process directly."
                )
            if out.get("host_build") is not None:
                from .containers.build_grant import HostBuildGrant

                grant = HostBuildGrant.from_value(out["host_build"])
                out["host_build"] = (
                    out["host_build"]
                    if isinstance(out["host_build"], bool)
                    else (grant.to_dict() if grant is not None else None)
                )
            if out.get("allowed_host_env"):
                from .containers import validate_host_env_patterns

                validate_host_env_patterns(out["allowed_host_env"], source="allowed_host_env")
            if out.get("build") is not None or out.get("dockerfile") is not None:
                from .containers.build_grant import resolve_direct_host_build

                out["build"] = resolve_direct_host_build(
                    {"build": out.get("build"), "dockerfile": out.get("dockerfile")},
                    allowed_host_paths=tuple(out.get("allowed_host_paths") or ()),
                    allowed_host_env=tuple(out.get("allowed_host_env") or ()),
                    host_build=out.get("host_build"),
                )
            if "execution_mode" not in out and (
                out.get("build") is not None
                or any(
                    isinstance(out.get(k), str) and out[k].strip()
                    for k in ("compose_file", "image", "dockerfile")
                )
            ):
                out["execution_mode"] = "container"
            return out
        return data

    @model_validator(mode="after")
    def _validate_container_mode_source(self) -> CapsemSandboxConfig:
        if (
            self.execution_mode == "container"
            and not (self.image and self.image.strip())
            and not (self.compose_file and self.compose_file.strip())
            and not (self.dockerfile and self.dockerfile.strip())
            and not self.build
        ):
            raise ValueError(
                "execution_mode='container' requires an explicit OCI image reference "
                "(set 'image', 'compose_file', 'dockerfile', or 'build')."
            )
        return self

    execution_mode: Literal["vm", "container"] = "vm"
    image: str | None = None
    cpu_count: int = Field(default=4)
    ram_gb: int = Field(default=8)
    working_dir: str | None = Field(default=None)
    compose_file: str | None = None
    dockerfile: str | None = None
    build: dict[str, Any] | None = None
    host_build: HostBuildGrant | bool | None = None
    environment: dict[str, str] = Field(default_factory=dict)
    command: tuple[str, ...] | str | None = None
    volumes: tuple[str, ...] = ()
    healthcheck: dict[str, Any] | None = None
    mem_limit: str | None = None
    user: str | None = Field(default=None)
    allowed_host_env: tuple[str, ...] = ()
    allowed_host_paths: tuple[str, ...] = ()

    def __hash__(self) -> int:
        return hash(tuple(_json_hashable(getattr(self, k)) for k in type(self).model_fields))

    def to_container_spec(self) -> ContainerSpec:
        """Convert this sandbox config into an Inspect-free `ContainerSpec`."""
        from .containers import ContainerSpec, resolve_effective_allowed_host_paths

        if not (self.image and self.image.strip()) and not self.build:
            raise ValueError(
                "execution_mode='container' requires an explicit OCI image reference "
                "(set 'image', 'build', or a Compose service 'image')."
            )
        eff_paths = list(resolve_effective_allowed_host_paths(self.allowed_host_paths))
        if self.compose_file and self.compose_file.strip():
            compose_dir = str(Path(self.compose_file).expanduser().resolve().parent)
            if compose_dir not in eff_paths:
                eff_paths.append(compose_dir)
        return ContainerSpec(
            image=self.image,
            working_dir=self.working_dir or "/workspace",
            working_dir_explicit=self.working_dir is not None,
            environment=dict(self.environment),
            command=self.command,
            volumes=self.volumes,
            healthcheck=dict(self.healthcheck) if self.healthcheck is not None else None,
            mem_limit=self.mem_limit,
            user=self.user,
            allowed_host_paths=tuple(eff_paths),
            build=dict(self.build) if self.build is not None else None,
        )
