"""What `config/gate.toml` says about linting: trees, tools and surfaces.

Split from `harnessschema`, which describes the gate running itself. The seam
is the same one that file already documents: machinery there, what the
machinery checks here. Keeping both put `harnessschema` past the module ceiling
this package enforces on its own source.
"""

from __future__ import annotations

from pathlib import PurePosixPath

from pydantic import Field, PositiveInt, field_validator, model_validator

from .configschema import Strict
from .harnessschema import PythonRoot, SuppressionBudget, TyRule


class MarkdownLintConfig(Strict):
    """Documented promises that do not resolve yet, as exact debt.

    Keyed `document|target`, valued with the reason. A document telling a
    reader -- usually an agent -- to open a file that is not there is worse
    than saying nothing, so a new one fails.

    The value is a reason rather than a bare `true` because a suppression
    nobody can evaluate is a suppression nobody removes. An entry that no
    longer applies is stale and fails too: an inventory that drifts from the
    tree has stopped ratcheting.
    """

    known_missing_targets: dict[str, str] = Field(default_factory=dict)


class LintConfig(Strict):
    """Which trees are checked, which strictly, and what is held back."""

    markdown: MarkdownLintConfig = MarkdownLintConfig()
    python_roots: tuple[PythonRoot, ...]
    strict_roots: tuple[PythonRoot, ...]
    python_platform: str
    """One explicit Ty platform so the exact ratchet is host-independent."""

    error_on_warning: bool = True
    """A `ty` warning exits zero, so a warning-level rule on the ratchet could
    never be observed as fixed. Semantic policy; how the pinned tool spells
    the flag belongs to the adapter."""

    ty_ratchet: dict[TyRule, PositiveInt]
    """Exact relaxed-tree diagnostic counts, keyed by Ty rule."""

    suppression_budget: SuppressionBudget

    @property
    def relaxed_roots(self) -> tuple[str, ...]:
        return tuple(name for name in self.python_roots if name not in self.strict_roots)

    @field_validator("python_roots", "strict_roots")
    @classmethod
    def _no_duplicates(cls, values: tuple[str, ...]) -> tuple[str, ...]:
        """A repeated root checks a tree twice and obscures its ownership."""
        seen = [value for value in values if values.count(value) > 1]
        if seen:
            raise ValueError(f"duplicated: {', '.join(sorted(set(seen)))}")
        return values

    @field_validator("python_roots", "strict_roots")
    @classmethod
    def _stay_inside_the_checkout(cls, values: tuple[str, ...]) -> tuple[str, ...]:
        for value in values:
            parts = PurePosixPath(value)
            if parts.is_absolute() or ".." in parts.parts:
                raise ValueError(f"{value!r} must be relative and must not escape upwards")
        return values

    @model_validator(mode="after")
    def _strict_roots_are_checked_at_all(self) -> LintConfig:
        """A strict root nobody checks silently checks nothing, and reads as
        passing."""
        stray = set(self.strict_roots) - set(self.python_roots)
        if stray:
            raise ValueError(
                f"strict_roots must be a subset of python_roots; {', '.join(sorted(stray))} "
                "would be checked strictly and never checked at all"
            )
        return self


class LintSurface(Strict):
    """A kind of first-party file, and the steps that must check it."""

    name: str
    pattern: str
    enforced_by: tuple[str, ...]
    """Steps that must exist *and* answer in the fast phase.

    A lint needs no build, so a lint that reports after the asset build is a
    lint that reported after the expensive work it should have preceded.
    """

    checked_by: tuple[str, ...] = ()
    """Steps that must exist, in any phase.

    For checks that legitimately cannot be early. Rust tests need a compiled
    workspace, so they belong beside the coverage run and not beside Ruff --
    but "runs late" and "does not run at all" are different facts, and only
    the first one is acceptable. Without this the inventory recorded which
    surfaces were *linted* and said nothing about which were *tested*.
    """
