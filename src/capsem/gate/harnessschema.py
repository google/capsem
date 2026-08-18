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
    PositiveFloat,
    PositiveInt,
    StringConstraints,
    field_validator,
    model_validator,
)

from .configschema import Strict
from .digestschema import DigestConfig, LedgerConfig
from .exclusions import Exclusion, HashedExclusion
from .prefixschema import PrefixConfig as PrefixConfig
from .sandboxschema import SandboxConfig as SandboxConfig
from .sourcecontractschema import ScriptSizeConfig
from .timingschema import TimingRegressionConfig as TimingRegressionConfig

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


class ShellBodyConfig(Strict):
    max_lines: int
    oversized_line_counts: dict[str, int]


class StepAttributeConfig(Strict):
    """The step-attribute migration ledger.

    `max_undeclared` is the destination, not the current state: it is zero, and
    `undeclared_by_module` is the exact remaining debt that may only shrink.
    """

    max_undeclared: NonNegativeInt
    undeclared_by_module: dict[str, int]


class BoundaryConfig(Strict):
    max_recipe_lines: int
    max_module_lines: int
    #: One rule per first-party source family. `scripts` and `rust` are the
    #: same shape -- roots, suffixes, a ceiling, an exact debt inventory --
    #: because they are the same rule asked of different trees.
    scripts: ScriptSizeConfig
    rust: ScriptSizeConfig
    #: Shell bodies are measured, not listed: they live inside YAML and
    #: Dockerfiles rather than in files of their own, so there is no root or
    #: suffix to declare.
    shell_bodies: ShellBodyConfig
    step_attributes: StepAttributeConfig
    #: Steps that drive cargo without claiming the workspace, each saying
    #: why. Not a bare list of names: one comment over an unbounded list
    #: stops being a reason at the second entry.
    unclaimed_cargo: tuple[Exclusion, ...]
    #: `command || true`: a verdict deliberately thrown away, pinned to the
    #: hash of the parsed command so the ledger tracks the decision, not the
    #: formatting.
    discarded_verdicts: tuple[HashedExclusion, ...]
    #: Dockerfile `RUN` bodies that sequence several statements on purpose
    #: without `set -e`, each stating why the earlier failures are tolerable.
    sequenced_runs: tuple[Exclusion, ...]
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

    cancellation_grace_seconds: PositiveFloat
    cancellation_poll_seconds: PositiveFloat

    @model_validator(mode="after")
    def _name_exclusives(self) -> ExecutionConfig:
        for key, exclusive in self.exclusives.items():
            if not exclusive.name:
                object.__setattr__(exclusive, "name", key)
        return self


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
    #: Trees where identical bytes are a third party's doing, not ours.
    duplicate_content_exempt: tuple[str, ...]
    #: Exact frozen source replicas: copied inputs, not authored artifacts.
    source_replica_roots: tuple[str, ...]
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
    source_archive_dir: str
    keep_runs: PositiveInt
    keep_bytes: PositiveInt
    artifact_digest: str
    slow_action_seconds: NonNegativeFloat
    failure_tail_lines: PositiveInt
    timing_regression: TimingRegressionConfig
    #: The distilled history that outlives the directories above, and the
    #: overview computed from it. In `digestschema` because this module is at
    #: its own line boundary and thresholds are what people come to tune.
    ledger: LedgerConfig
    digest: DigestConfig

    @field_validator("source_archive_dir")
    @classmethod
    def _archive_is_one_relative_directory(cls, value: str) -> str:
        if PurePosixPath(value).name != value or value in {".", ".."}:
            raise ValueError("source_archive_dir must be one relative directory name")
        return value

    @model_validator(mode="after")
    def _archive_does_not_alias_run_metadata(self) -> RunLogConfig:
        if self.source_archive_dir in {self.latest_link, self.history_lock}:
            raise ValueError("source_archive_dir must not alias run-history metadata")
        return self


class DiskConfig(Strict):
    reclaimable: tuple[str, ...]
    required_free_gb: int
    required_free_scratch_gb: int
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

    @field_validator("run_dir")
    @classmethod
    def _run_dir_is_short_absolute_template(cls, template: str) -> str:
        path = PurePosixPath(template)
        if not path.is_absolute() or template.count("{root_id}") != 1:
            raise ValueError("workspace run_dir must be absolute and contain {root_id} once")
        return template
