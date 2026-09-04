"""Stable private-checkout identities derived from the source being tested."""

from __future__ import annotations

from pathlib import Path

from . import snapshot
from .config import GateConfig
from .prefixlease import parent_dir
from .sourcecommit import SourceCommit


def example(config: GateConfig) -> Path:
    """A representative prefix path for path-length arithmetic."""
    return parent_dir(config) / ("0" * 40)


def for_source_commit(config: GateConfig, commit: SourceCommit) -> Path:
    """The deterministic release prefix: the complete source identity."""
    return parent_dir(config) / str(commit)


def for_working_tree(config: GateConfig) -> Path:
    """The stable private path for the checkout's exact source bytes."""
    identity = snapshot.digest(config.root, config)
    return parent_dir(config) / identity[: config.prefix.name_length]
