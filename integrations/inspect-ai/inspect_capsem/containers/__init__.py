"""Inspect-agnostic container tooling for Capsem VMs.

Provides Compose YAML parsing, Dockerfile rewriting and CA injection, build context packing
and host image caching, and container runtime orchestration. Must not import `inspect_ai`
or the rest of `inspect_capsem`.
"""

from __future__ import annotations

from inspect_capsem.containers.compose import (
    extract_compose_fields,
    parse_compose_yaml_file,
)
from inspect_capsem.containers.controller import CapsemController, CommandResult
from inspect_capsem.containers.dockerfile import (
    lower_dockerfile_heredocs,
    patch_dockerfile_for_capsem_ca,
    resolve_dockerfile_defaults,
)
from inspect_capsem.containers.image_cache import pack_build_context
from inspect_capsem.containers.runtime import (
    prepare_oci_workload_container,
    start_container_for_init,
)
from inspect_capsem.containers.spec import ContainerSpec

__all__ = [
    "CapsemController",
    "CommandResult",
    "ContainerSpec",
    "extract_compose_fields",
    "lower_dockerfile_heredocs",
    "pack_build_context",
    "parse_compose_yaml_file",
    "patch_dockerfile_for_capsem_ca",
    "prepare_oci_workload_container",
    "resolve_dockerfile_defaults",
    "start_container_for_init",
]
