"""Configuration vocabulary for repository source-shape contracts.

Kept apart from the runtime harness schema because source inventories are a
different responsibility, and because the gate enforces its own 300-line
module boundary.
"""

from __future__ import annotations

from pathlib import PurePosixPath
from typing import Annotated

from pydantic import PositiveInt, StringConstraints, field_validator, model_validator

from .configschema import Strict

#: A relative source path owned by this checkout. The size contract passes
#: these values to Git, so absolute or upward paths would inspect source that
#: is not part of the repository being qualified.
SourcePath = Annotated[str, StringConstraints(min_length=1, pattern=r"^[A-Za-z0-9_][\w./-]*$")]

#: A conventional script suffix. Suffix-less executables are found by their
#: shebang; this vocabulary finds library-style script modules as well.
ScriptSuffix = Annotated[str, StringConstraints(pattern=r"^\.[a-z0-9]+$")]


class ScriptSizeConfig(Strict):
    """The new-script ceiling and exact inventory of larger historical debt."""

    roots: tuple[SourcePath, ...]
    suffixes: tuple[ScriptSuffix, ...]
    max_lines: PositiveInt
    oversized_line_counts: dict[SourcePath, PositiveInt]

    @field_validator("roots", "suffixes")
    @classmethod
    def _no_duplicate_scope_values(cls, values: tuple[str, ...]) -> tuple[str, ...]:
        duplicates = sorted({value for value in values if values.count(value) > 1})
        if duplicates:
            raise ValueError(f"duplicated: {', '.join(duplicates)}")
        return values

    @field_validator("roots")
    @classmethod
    def _roots_stay_inside_the_checkout(cls, values: tuple[str, ...]) -> tuple[str, ...]:
        for value in values:
            path = PurePosixPath(value)
            if path.is_absolute() or ".." in path.parts:
                raise ValueError(f"{value!r} must be relative and must not escape upwards")
        return values

    @model_validator(mode="after")
    def _ratchets_are_oversized_scripts_in_scope(self) -> ScriptSizeConfig:
        roots = tuple(PurePosixPath(root) for root in self.roots)
        for value, line_count in self.oversized_line_counts.items():
            path = PurePosixPath(value)
            if path.is_absolute() or ".." in path.parts:
                raise ValueError(f"{value!r} must be relative and must not escape upwards")
            if not any(path.is_relative_to(root) for root in roots):
                raise ValueError(f"{value!r} is outside configured script roots")
            if line_count <= self.max_lines:
                raise ValueError(
                    f"{value!r} no longer exceeds the {self.max_lines}-line ceiling; "
                    "remove its stale ratchet"
                )
        return self
