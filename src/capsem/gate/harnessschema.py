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

from pydantic import NonNegativeFloat, PositiveInt, field_validator, model_validator

from .configschema import Strict


class LintConfig(Strict):
    python_roots: tuple[str, ...]
    strict_roots: tuple[str, ...]
    ty_flags: tuple[str, ...]
    ty_ratchet: tuple[str, ...]

    @property
    def relaxed_roots(self) -> tuple[str, ...]:
        return tuple(name for name in self.python_roots if name not in self.strict_roots)

class BoundaryConfig(Strict):
    max_recipe_lines: int
    max_module_lines: int
    shell_control_flow: tuple[str, ...]
    recipes_with_inline_control_flow: tuple[str, ...]
    direct_machine_access: tuple[str, ...]
    direct_concurrency: tuple[str, ...]
    modules_bypassing_primitives: tuple[str, ...]

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
    history_lock: str
    active_marker: str
    keep_runs: PositiveInt
    keep_bytes: PositiveInt
    artifact_digest: str
    slow_action_seconds: NonNegativeFloat

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

class WorkspaceConfig(Strict):
    home: str
    run_dir: str
    seeded_dirs: tuple[str, ...]
    benchmark_root: str
    coverage_file: str
    evidence_dir: str
