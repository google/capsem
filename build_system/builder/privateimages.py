"""Pure identities for Capsem's private build-helper image generations."""

from __future__ import annotations

import re
from dataclasses import dataclass

SOURCE_LABEL = "org.opencontainers.image.source"


@dataclass(frozen=True)
class PrivateImage:
    """One local helper and its immutable private registry generation."""

    local: str
    platform: str
    generation: str
    identity_label: str
    identity: str

    def repository(self, registry: str) -> str:
        local_repository = self.local.rsplit(":", 1)[0]
        platform = self.platform.replace("/", "-")
        if re.fullmatch(r"[a-z0-9][a-z0-9._-]*", local_repository) is None:
            raise ValueError(f"private image local repository is unsafe: {local_repository!r}")
        return f"{registry}/{local_repository}-{platform}"

    def remote(self, registry: str) -> str:
        return f"{self.repository(registry)}:{self.generation}"

    def package(self, registry: str) -> str:
        registry_host = registry.split("/", 1)[0]
        return self.repository(registry).removeprefix(f"{registry_host}/")
