"""Config-derived fixture materialization for complete profile content."""

from __future__ import annotations

from collections.abc import Iterable
from pathlib import Path

from capsem.gate.config import GateConfig
from capsem.gate.configschema import Arch


def materialize_required_artifacts(
    config: GateConfig,
    assets: Path,
    *,
    arches: Iterable[Arch] | None = None,
) -> None:
    """Write every artifact that makes a selected architecture complete."""
    selected = tuple(config.architectures.values()) if arches is None else tuple(arches)
    for arch in selected:
        directory = assets / arch.name
        directory.mkdir(parents=True, exist_ok=True)
        for name in (*config.artifacts.bootable, *config.assets.evidence_artifacts):
            (directory / name).write_bytes(b"fixture artifact\n")
