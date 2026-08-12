"""Closed Docker network vocabularies shared by builders and release gates."""

from __future__ import annotations

from enum import StrEnum


class BuildNetwork(StrEnum):
    """Network modes accepted by ``docker build`` and BuildKit."""

    DEFAULT = "default"
    HOST = "host"
    NONE = "none"


class ContainerNetwork(StrEnum):
    """Network modes accepted by ``docker run`` and ``docker create``."""

    BRIDGE = "bridge"
    HOST = "host"
    NONE = "none"


def require_build_network(network: BuildNetwork) -> str:
    """Refuse strings and container-only modes at the command boundary."""
    if not isinstance(network, BuildNetwork):
        raise TypeError("Docker builds accept only BuildNetwork enum members")
    return network.value


def require_container_network(network: ContainerNetwork) -> str:
    """Refuse strings and BuildKit-only modes at the command boundary."""
    if not isinstance(network, ContainerNetwork):
        raise TypeError("Docker containers accept only ContainerNetwork enum members")
    return network.value
