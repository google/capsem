"""One ordering contract for package provenance in every release adapter."""

from __future__ import annotations

from collections.abc import Callable
from pathlib import Path
from typing import Protocol, TypeVar

from .productschema import ProfileRevisionPolicy
from .sourcecommit import SourceCommit

Manifest = TypeVar("Manifest")


class Runner(Protocol):
    def __call__(self, command: list[str], *, env: dict[str, str] | None = None) -> None: ...


def author_binary_graph(
    source: Manifest,
    *,
    build: Callable[[Manifest], Manifest],
    record: Callable[[Manifest], None],
) -> Manifest:
    """Convert to a graph, stamp package provenance, then rebuild catalogs.

    ``source`` may be the legacy runtime projection used by local profile
    assets. Package provenance belongs only on release-graph package rows, so
    recording before the first build is invalid. The second build validates
    the mutated graph and regenerates every derived channel document.
    """
    graph = build(source)
    record(graph)
    return build(graph)


def author_native_candidate(
    source: Path,
    *,
    runner: Runner,
    admin: Path,
    assets_dir: Path,
    profiles_dir: Path,
    channel: str,
    version: str,
    source_commit: SourceCommit,
    artifacts: tuple[Path, ...],
    release_environment: dict[str, str],
    asset_source_base: str,
    dist: Path,
    graph_manifest: Path,
    manifest_version: str,
    profile_revision_policy: ProfileRevisionPolicy | None = None,
) -> Path:
    """Author one native package graph through the shared graph-first order."""

    def build(manifest: Path) -> Path:
        command = [
            str(admin),
            "assets",
            "channel",
            "build",
            "--manifest",
            manifest.resolve().as_uri(),
            "--assets-dir",
            str(assets_dir),
            "--profiles-dir",
            str(profiles_dir),
            "--channel",
            channel,
            "--manifest-version",
            manifest_version,
            "--asset-source-base",
            asset_source_base,
            "--out-dir",
            str(dist),
        ]
        if profile_revision_policy is not None:
            command.extend(("--profile-revision-policy", profile_revision_policy.value))
        runner(command, env=release_environment)
        return graph_manifest

    def record(manifest: Path) -> None:
        command = [
            str(admin),
            "assets",
            "channel",
            "record-binary",
            "--manifest-path",
            str(manifest),
            "--version",
            version,
            "--source-commit",
            str(source_commit),
        ]
        for artifact in artifacts:
            command.extend(("--artifact", str(artifact)))
        runner(command, env=release_environment)

    return author_binary_graph(source, build=build, record=record)
