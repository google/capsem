"""Compose data supplied by an evaluator; no host discovery or authority."""

from __future__ import annotations

import posixpath
from collections.abc import Mapping
from dataclasses import dataclass, field
from pathlib import PurePath
from types import MappingProxyType

from .dockerfile_metadata import _extract_dockerfile_metadata


@dataclass(frozen=True)
class ComposeInputs:
    environment: Mapping[str, str] = field(default_factory=dict)
    files: Mapping[str, str] = field(default_factory=dict)
    directories: frozenset[str] = frozenset()

    def __post_init__(self) -> None:
        object.__setattr__(self, "environment", MappingProxyType(dict(self.environment)))
        object.__setattr__(
            self,
            "files",
            MappingProxyType({self.resolve(key): value for key, value in self.files.items()}),
        )
        object.__setattr__(
            self, "directories", frozenset(self.resolve(path) for path in self.directories)
        )

    @staticmethod
    def resolve(path: str | PurePath) -> str:
        """Lexical normalization, without cwd, symlink resolution or stat."""
        return posixpath.normpath(str(path))

    def is_dir(self, path: str | PurePath) -> bool:
        return self.resolve(path) in self.directories

    def exists(self, path: str | PurePath) -> bool:
        key = self.resolve(path)
        return key in self.files or key in self.directories

    def dockerfile_defaults(self, path: str | PurePath) -> dict[str, str]:
        text = self.files.get(self.resolve(path))
        if text is None:
            return {}
        workdir, user = _extract_dockerfile_metadata(text)
        return {
            key: value
            for key, value in [("working_dir", workdir), ("user", user)]
            if value is not None
        }


EMPTY_INPUTS = ComposeInputs()


@dataclass(frozen=True)
class ComposeLimits:
    maximum_bytes: int
    maximum_nodes: int
    maximum_depth: int

    def __post_init__(self) -> None:
        if any(
            type(value) is not int or value <= 0
            for value in [self.maximum_bytes, self.maximum_nodes, self.maximum_depth]
        ):
            raise ValueError("Compose limits must be positive integers")


class InterpolationBudget:
    """Bound intermediate expanded fragments as well as the output tree."""

    def __init__(self, limits: ComposeLimits):
        self.limits = limits
        self.remaining_bytes = limits.maximum_bytes
        self.remaining_nodes = limits.maximum_nodes

    def depth(self, depth: int) -> None:
        if depth > self.limits.maximum_depth:
            raise ValueError("Compose depth limit exceeded")

    def node(self, depth: int) -> None:
        self.depth(depth)
        self.remaining_nodes -= 1
        if self.remaining_nodes < 0:
            raise ValueError("Compose node limit exceeded")

    def consume(self, value: str) -> None:
        self.remaining_bytes -= len(value.encode("utf-8"))
        if self.remaining_bytes < 0:
            raise ValueError("Compose expanded byte limit exceeded")


class Fragments(list[str]):
    def __init__(self, budget: InterpolationBudget):
        super().__init__()
        self.budget = budget

    def append(self, value: str) -> None:
        self.budget.consume(value)
        super().append(value)
