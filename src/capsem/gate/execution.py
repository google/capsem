"""A named unit of gate work: some actions, what it may not share, what it makes.

Deliberately concrete. A `Step` has no behaviour of its own beyond running its
actions in order and recording what it produced, so there is nothing to
subclass. Polymorphism lives one level down in `Action`, where the variation
actually is; a `Step` subclass would only ever be a closure wearing a class,
which is the shape this replaces.

Ordering is deliberately absent. A step does not know what comes before it --
that is `plan`, which holds the edges. Keeping it out means the same step can
be reused by a command that sequences it differently, which is the whole reason
the six test modules can share one pytest step instead of eleven near-copies.

`contends` names what this step may not share, drawn from
`[execution.exclusives]` where each entry carries the reason it exists.
`produces` names the artifacts whose bytes the run log should record, so a run
can answer "which bytes did this build" after the tree is gone.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum, StrEnum, auto
from pathlib import Path

from .actions import Action
from .context import Context
from .fileactions import Hash
from .harnessschema import Exclusive


class ResumePolicy(StrEnum):
    """Whether a diagnostic continuation may reuse this step's result."""

    REUSE = "reuse"
    ALWAYS_RUN = "always-run"


#: A step that takes the whole machine. Not a core count: plan construction is
#: inert and may not read the host, so the resolution happens in the analysis
#: that knows how many cores the run had.
SATURATES = 0


class Kind(StrEnum):
    """What a step *is*, so the graph can be reasoned about rather than read.

    Declared rather than inferred from the label. Everything that tried to
    infer it -- a contract grepping for `fast.` in a name, a doc mapping stage
    titles to label prefixes -- was reading a naming convention and calling it
    a property, which is why renaming a step could silently change what was
    being checked.
    """

    LINT = "lint"
    STATIC_TEST = "static-test"
    COMPILE = "compile"
    UNIT_TEST = "unit-test"
    CAPSEM = "capsem"
    E2E = "e2e"
    PACKAGE = "package"
    PUBLISH = "publish"
    UNDECLARED = "undeclared"
    """Migration only. `[boundary.step_attributes]` inventories what is left."""


class Needs(StrEnum):
    """A capability a step requires. A step declares a *set* of these.

    A set rather than one classification, because the questions asked of it are
    independent: hermeticity is about `NETWORK`, contention is about `DOCKER`
    and `VM`, and a step can want both or neither.

    `NETWORK` earns its place twice. It decides hermeticity, and it means the
    step's duration is not a measurement of our own work -- so speed
    conformance has to widen its tolerance instead of reporting a slow mirror
    as a regression.
    """

    NETWORK = "network"
    DISK = "disk"
    DOCKER = "docker"
    VM = "vm"
    KVM = "kvm"
    SIGNING = "signing"


class Arch(Enum):
    """Which architecture a step's work belongs to.

    `ANY` is for work that is architecture-neutral; `HOST` is for work that is
    specifically about the machine running the gate. They are different claims,
    and conflating them is what lets a host-only step be scheduled into a
    cross-architecture lane.

    Alone among the vocabularies here, the members carry no value. `config.arch`
    already owns the spellings, and writing them again as enum values made a
    second list of architectures -- the precise shape of the bug that
    `[architectures]` was centralised to end. The literal-data guard said so,
    and it was right. Compare members, never strings;
    `tests/test_gate_has_no_literal_data.py` holds the member names to config.
    """

    HOST = auto()
    X86_64 = auto()
    ARM64 = auto()
    ANY = auto()


class Speed(StrEnum):
    """Whether a step is cheap *relative to the work its lane protects*.

    Not an absolute duration, and deliberately not `slow_action_seconds` --
    that is a reporting threshold for naming actions in the timing summary, and
    borrowing it here would call a two-minute step slow.

    Relative is the only reading that means anything. The fast phase runs about
    four minutes and exists so a lint error fails before a candidate that runs
    about a hundred and forty. A two-minute step inside it is a three percent
    tax on catching a typo early, which is the trade the phase was created to
    make. The same two minutes sitting between a VM boot and a package build
    would be noise.

    So `FAST` means: proportionate to the lane it is in. What the invariant
    checks is not a per-step second count but whether a lane has stopped being
    cheap compared with what comes after it.
    """

    FAST = "fast"
    SLOW = "slow"
    UNDECLARED = "undeclared"


class Requires(StrEnum):
    """Why an edge exists, which decides whether it may be removed.

    A redundant `ARTIFACT` edge is usually harmless -- the consumer really does
    need those bytes and the extra edge only restates it. A redundant `ORDER`
    edge is lost parallelism: two steps sequenced that the graph already
    sequenced, and the machine idles for it.
    """

    ARTIFACT = "artifact"
    ORDER = "order"
    EVIDENCE = "evidence"
    UNDECLARED = "undeclared"
    """Until `Plan.add` carries the kind. Inferring it from whether the
    predecessor `produces` anything would be a guess wearing a type, and the
    guess would be wrong exactly where it matters -- an `ORDER` edge added
    defensively next to a real `ARTIFACT` one."""


@dataclass(frozen=True)
class Step:
    """One named unit of gate work."""

    label: str
    actions: tuple[Action, ...]
    contends: tuple[Exclusive, ...] = ()
    produces: tuple[Path, ...] = field(default_factory=tuple)
    carry_checks: tuple[Action, ...] = ()
    resume: ResumePolicy = ResumePolicy.REUSE

    # -- what this step is, for the graph ---------------------------------
    #
    # Defaulted only while the 131 existing call sites are migrated;
    # `[boundary.step_attributes]` holds the exact remaining count and a
    # citadel guard refuses to let it grow. They become required arguments
    # when it reaches zero.
    kind: Kind = Kind.UNDECLARED
    needs: frozenset[Needs] = frozenset()
    arch: Arch = Arch.ANY
    speed: Speed = Speed.UNDECLARED
    concurrency: int = 1
    """Workers this step actually uses, or `SATURATES` for all of them.

    Separate from `contends`, which says what may not overlap and nothing
    about how much machine is consumed. Without it the width of an antichain
    is a step count rather than a load, and a lane of eight single-threaded
    steps looks identical to eight that each want every core.

    A core count cannot be written here: plan construction is inert and must
    not read the machine, so `SATURATES` is resolved against `cores` by
    whoever is doing the arithmetic.
    """

    @property
    def declared(self) -> bool:
        """Whether this step has been through the attribute migration."""
        return self.kind is not Kind.UNDECLARED and self.speed is not Speed.UNDECLARED

    def render(self) -> list[str]:
        """One line per action, for the dry run."""
        return [action.render() for action in self.actions]

    def run(self, context: Context) -> None:
        """Every action in order, then record what came out.

        Stops at the first failing action: the ones after it were written
        against what it was supposed to produce.
        """
        for action in self.actions:
            if context.watch is not None:
                context.watch.checkpoint()
            with context.journal.action(action):
                action.perform(context)
            if context.watch is not None:
                context.watch.checkpoint()
        for artifact in self.produces:
            if context.watch is not None:
                context.watch.checkpoint()
            # Bracketed like every other action. Hashing a multi-gigabyte
            # rootfs is real time, and outside the bracket it was time the
            # timing report could not see -- so "the gate is slow" resolved to
            # a step with no line in it accounting for the difference.
            hashing = Hash(artifact)
            with context.journal.action(hashing):
                hashing.perform(context)
            if context.watch is not None:
                context.watch.checkpoint()


def step(
    label: str,
    *actions: Action,
    contends: tuple[Exclusive, ...] = (),
    produces: tuple[Path, ...] = (),
    carry_checks: tuple[Action, ...] = (),
    resume: ResumePolicy = ResumePolicy.REUSE,
    kind: Kind = Kind.UNDECLARED,
    needs: frozenset[Needs] = frozenset(),
    arch: Arch = Arch.ANY,
    speed: Speed = Speed.UNDECLARED,
    concurrency: int = 1,
) -> Step:
    """Build a step from actions given positionally, which reads better.

        step("sign", Run([...]), Run([...]))

    rather than passing a tuple, because at every call site the actions are
    written out literally and the extra brackets are noise.
    """
    return Step(
        label=label,
        actions=actions,
        contends=contends,
        produces=produces,
        carry_checks=carry_checks,
        resume=resume,
        kind=kind,
        needs=needs,
        arch=arch,
        speed=speed,
        concurrency=concurrency,
    )
