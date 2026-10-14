"""Inspect-free OCI container and Compose support for `inspect-capsem`."""

from __future__ import annotations

from .build_grant import HostBuildGrant
from .compose import extract_compose_fields, parse_compose_yaml_file, parse_host_compose_yaml_file
from .compose_fields import (
    extract_capsem_compose_fields,
    normalize_volumes,
    resolve_effective_allowed_host_paths,
)
from .compose_inputs import validate_host_env_patterns
from .controller import ContainerCommandResult, ContainerController
from .runtime import prepare_oci_workload_container, resolve_container_working_dir
from .spec import ContainerSpec

__all__ = [
    "ContainerCommandResult",
    "ContainerController",
    "ContainerSpec",
    "HostBuildGrant",
    "extract_capsem_compose_fields",
    "extract_compose_fields",
    "normalize_volumes",
    "parse_compose_yaml_file",
    "parse_host_compose_yaml_file",
    "prepare_oci_workload_container",
    "resolve_container_working_dir",
    "resolve_effective_allowed_host_paths",
    "validate_host_env_patterns",
]
