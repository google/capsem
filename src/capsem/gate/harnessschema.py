"""What `config/gate.toml` says about the gate running itself.

`configschema` describes the product: architectures, packages, assets, the
install proof. This describes the machinery -- what may not run beside what,
who holds the machine, where a run is recorded, how much disk it may occupy,
and the limits the gate holds its own source to.

Split from `configschema` because the two answer different questions and a
single file carrying both was already past the module ceiling that this project
enforces on itself. The `Strict` base is shared, so a typo is still a failure
and not a silent default in either half.
"""

from __future__ import annotations

from pathlib import PurePosixPath
from typing import Annotated

from pydantic import (
    NonNegativeFloat,
    NonNegativeInt,
    PositiveInt,
    StringConstraints,
    field_validator,
    model_validator,
)

from .configschema import Strict

#: A first-party tree to check. Relative, normalized, and inside the checkout:
#: an absolute or escaping root would check somebody else's code and report it
#: as this repository's.
PythonRoot = Annotated[str, StringConstraints(min_length=1, pattern=r"^[A-Za-z0-9_][\w./-]*$")]

#: A `ty` diagnostic name. The grammar only -- third-party vocabularies change,
#: so mirroring every rule in an enum dates the moment the tool is bumped.
#: What matters is that a *typo* is refused: `ty` ignores an unknown
#: `--ignore`, so a misspelt entry held nothing back and looked exactly like a
#: rule somebody had fixed.
TyRule = Annotated[str, StringConstraints(pattern=r"^[a-z][a-z0-9]*(?:-[a-z0-9]+)+$")]


class SuppressionBudget(Strict):
    """Exact Python-analysis debt that may only shrink deliberately."""

    noqa: NonNegativeInt
    type_ignore: NonNegativeInt
    ty_ignore: NonNegativeInt
    ruff_global_ignore: NonNegativeInt
    ruff_per_file_ignore: NonNegativeInt
    justification: Annotated[str, StringConstraints(min_length=20)]


class LintConfig(Strict):
    """Which trees are checked, which strictly, and what is held back."""

    python_roots: tuple[PythonRoot, ...]
    strict_roots: tuple[PythonRoot, ...]

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


class BoundaryConfig(Strict):
    max_recipe_lines: int
    max_module_lines: int
    shell_control_flow: tuple[str, ...]
    recipes_with_inline_control_flow: tuple[str, ...]
    direct_machine_access: tuple[str, ...]
    direct_concurrency: tuple[str, ...]


class Exclusive(Strict):
    """Something only one step may hold at a time, and why.

    The reason is not decoration. Written as `&` and `wait` in shell, this
    knowledge lived in comments beside the backgrounded job, and an eighth lane
    could violate a constraint recorded three hundred lines away.

    This is the only representation of an exclusive. A dataclass beside it
    would be a second place for one fact to live, which is how the architecture
    mapping ended up spelled four ways. Frozen, so it is hashable and can key
    the lock table the plan builds.
    """

    name: str = ""
    """Filled in at load from the table key that names it."""

    reason: str

    shared: bool = False
    """Whether this claim admits other shared claims on the same thing.

    Not a property of the resource -- a property of *this* step's claim on it.
    The asset lanes hold Docker shared, because they must overlap to fit the
    time budget; every other Docker step holds it exclusively, because it must
    not run beside them. One resource, two kinds of holder, which is a
    readers-writer lock.

    Without this, declaring the resource serialized the lanes and omitting it
    let anything schedule beside them -- so `assetlanes` grew a thread pool the
    plan could not see, order against, or attribute a failure to.
    """

    def held_shared(self) -> Exclusive:
        return self.model_copy(update={"shared": True})


class ExecutionConfig(Strict):
    exclusives: dict[str, Exclusive]

    max_parallel_steps: PositiveInt
    """How many steps may be in flight at once.

    An operational limit, so it is configuration; what a claim means and when
    one may be taken is scheduling semantics, so that is code.
    """

    @model_validator(mode="after")
    def _name_exclusives(self) -> ExecutionConfig:
        for key, exclusive in self.exclusives.items():
            if not exclusive.name:
                object.__setattr__(exclusive, "name", key)
        return self


class LockConfig(Strict):
    """One holder at a time, proven by the kernel rather than by a PID file."""

    path: str
    holder_record: str
    report_after_seconds: float
    wait_timeout_seconds: float
    poll_interval_seconds: float
    run_marker: str


class LocksConfig(Strict):
    gate: LockConfig


class RunLogConfig(Strict):
    """Retention that keeps nothing prunes the run being written.

    The failure then surfaces as a missing directory somewhere downstream
    rather than as the bad policy it is, so the bounds are on the types.
    """

    root: str
    events: str
    event_schema: str
    step_log_dir: str
    summary: str
    latest_link: str
    #: Trees each run watches for filesystem faults, relative to the checkout.
    observed_roots: tuple[str, ...]
    #: How Linux names the path behind a file descriptor, so a `dir_fd`-relative
    #: call can be anchored instead of resolved against the working directory.
    fd_path_template: str
    #: Where those faults are written the instant they are found, per run.
    error_log: str
    #: Size cap and generations kept, so faults cannot fill the disk.
    error_log_max_bytes: int
    error_log_keep: int
    history_lock: str
    active_marker: str
    keep_runs: PositiveInt
    keep_bytes: PositiveInt
    artifact_digest: str
    slow_action_seconds: NonNegativeFloat
    failure_tail_lines: PositiveInt


class DiskConfig(Strict):
    reclaimable: tuple[str, ...]
    required_free_gb: int
    run_footprint_warn_gb: int

    @field_validator("reclaimable")
    @classmethod
    def _stay_inside_the_checkout(cls, paths: tuple[str, ...]) -> tuple[str, ...]:
        """A reclaimer that can be aimed outside the checkout is a delete command.

        Checked at load, with the offending entry named, rather than trusted at
        the call site: the reclaimer removes whole trees, and the difference
        between a relative path and one that escapes upwards is one editing
        mistake.
        """
        for path in paths:
            parts = PurePosixPath(path)
            if parts.is_absolute() or ".." in parts.parts:
                raise ValueError(f"{path!r} must be relative and must not escape upwards")
        return paths


class PrefixConfig(Strict):
    """Where a run's private copy of the checkout lives, and what it carries."""

    parent: str
    name_length: int
    keep: int
    carried: tuple[str, ...]
    exports: tuple[str, ...]

    @model_validator(mode="after")
    def _paths_stay_inside(self) -> PrefixConfig:
        """`carried` and `exports` name places inside a checkout.

        An absolute entry would copy something the run does not own; a `..`
        entry would write outside the prefix on export, which is the one
        direction a private copy must never reach.
        """
        for group in (self.carried, self.exports):
            for path in group:
                parts = PurePosixPath(path)
                if parts.is_absolute() or ".." in parts.parts:
                    raise ValueError(f"{path!r} must be relative and must not escape upwards")
        return self


class WorkspaceConfig(Strict):
    home: str
    run_dir: str
    seeded_dirs: tuple[str, ...]
    benchmark_root: str
    coverage_file: str
    evidence_dir: str
