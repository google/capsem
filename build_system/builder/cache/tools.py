"""Digest-pinned executable tools materialized inside the cache authority."""

from __future__ import annotations

import hashlib
import os
import platform
import secrets
import shutil
import urllib.request
from collections.abc import Callable
from pathlib import Path
from typing import Annotated

from pydantic import (
    BaseModel,
    ConfigDict,
    Field,
    StrictBool,
    StrictInt,
    StrictStr,
    StringConstraints,
    model_validator,
)

from .paths import CachePaths

SafeToken = Annotated[StrictStr, StringConstraints(pattern=r"^[A-Za-z0-9][A-Za-z0-9._-]*$")]
Sha256 = Annotated[StrictStr, StringConstraints(pattern=r"^[0-9a-f]{64}$")]
HttpsUrl = Annotated[StrictStr, StringConstraints(pattern=r"^https://[^\s]+$")]
Download = Callable[[str, Path, int], None]


class ToolDistribution(BaseModel):
    """One checksum-authorized executable for a host platform."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    system: SafeToken
    machine: SafeToken
    url: HttpsUrl
    sha256: Sha256


class CachedToolPolicy(BaseModel):
    """The complete cache and supply-chain contract for one executable."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    name: SafeToken
    version: SafeToken
    cache_stage: SafeToken
    download_timeout_seconds: Annotated[StrictInt, Field(gt=0)]
    distributions: tuple[ToolDistribution, ...] = Field(min_length=1)

    @model_validator(mode="after")
    def distributions_are_unique(self) -> CachedToolPolicy:
        platforms = [(item.system, item.machine) for item in self.distributions]
        if len(platforms) != len(set(platforms)):
            raise ValueError("cached tool distributions must be unique by system and machine")
        return self


class MaterializedTool(BaseModel):
    """Typed result of resolving a pinned executable."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    path: Path
    sha256: Sha256
    cache_hit: StrictBool


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _download(url: str, destination: Path, timeout_seconds: int) -> None:
    request = urllib.request.Request(url, headers={"User-Agent": "capsem-tool-cache"})
    with (
        urllib.request.urlopen(request, timeout=timeout_seconds) as response,
        destination.open("wb") as output,
    ):
        shutil.copyfileobj(response, output)


def materialize(
    paths: CachePaths,
    policy: CachedToolPolicy,
    *,
    system: str | None = None,
    machine: str | None = None,
    download: Download = _download,
) -> MaterializedTool:
    """Resolve, verify, and atomically publish one configured executable."""
    selected_system = platform.system() if system is None else system
    selected_machine = platform.machine() if machine is None else machine
    matches = [
        item
        for item in policy.distributions
        if item.system == selected_system and item.machine == selected_machine
    ]
    if len(matches) != 1:
        raise ValueError(
            f"{policy.name} has no unique distribution for {selected_system}/{selected_machine}"
        )
    distribution = matches[0]
    target = (
        paths.stage(policy.cache_stage)
        / policy.name
        / policy.version
        / f"{selected_system}-{selected_machine}"
        / policy.name
    )
    if not target.is_symlink() and target.is_file() and _sha256(target) == distribution.sha256:
        return MaterializedTool(path=target, sha256=distribution.sha256, cache_hit=True)

    target.parent.mkdir(parents=True, exist_ok=True)
    temporary = target.with_name(f".{target.name}.{secrets.token_hex(6)}")
    try:
        download(distribution.url, temporary, policy.download_timeout_seconds)
        actual = _sha256(temporary)
        if actual != distribution.sha256:
            raise ValueError(
                f"{policy.name} {policy.version} checksum mismatch: "
                f"expected {distribution.sha256}, got {actual}"
            )
        os.chmod(temporary, 0o555)
        temporary.replace(target)
    finally:
        temporary.unlink(missing_ok=True)
    return MaterializedTool(path=target, sha256=distribution.sha256, cache_hit=False)
