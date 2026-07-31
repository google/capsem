"""Running gate steps, and refusing to run two that contend for one thing.

Concurrency in this gate is not a tuning knob. Some steps genuinely cannot
share the machine, and each reason is specific:

  the Apple VZ launch budget    two benchmark files launching VMs at once
                                measure each other, not Capsem

  the host save/restore lock    production has one service and one
                                service-scoped snapshot lock; an xdist worker
                                per service does not reproduce that

  the codesigned binaries       `cargo build --workspace` atomically replaces
                                the binaries a running VM test is using

Written as `&` and `wait` in shell, that knowledge lived in comments. A step
here declares what it `contends` for, and `run` refuses to schedule two
claimants of the same resource concurrently -- so the constraint is checked
rather than remembered.

Failures aggregate. A group with two broken steps reports two: stopping at the
first turns a second push into a second discovery, which is how a gate takes
three rounds to go green.
"""

from __future__ import annotations

from abc import ABC, abstractmethod
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field

from .errors import GateError


@dataclass(frozen=True)
class Exclusive:
    """Something only one step may hold at a time, and why."""

    name: str
    reason: str

    def __str__(self) -> str:
        return self.name


class Step(ABC):
    """One named unit of gate work."""

    def __init__(self, name: str, *, contends: tuple[Exclusive, ...] = ()) -> None:
        self.name = name
        self.contends = contends

    @abstractmethod
    def run(self) -> None:
        """Do the work. Raise to fail; the group reports which step it was."""

    @property
    def is_serial(self) -> bool:
        return bool(self.contends)


class Call(Step):
    """A step that invokes a callable, which is most of them."""

    def __init__(self, name: str, action, *, contends: tuple[Exclusive, ...] = ()) -> None:
        super().__init__(name, contends=contends)
        self._action = action

    def run(self) -> None:
        self._action()


@dataclass
class Outcome:
    """What a group did, in enough detail to say what to fix."""

    ran: list[str] = field(default_factory=list)
    failures: dict[str, BaseException] = field(default_factory=dict)

    @property
    def ok(self) -> bool:
        return not self.failures

    def raise_for_failures(self, what: str) -> None:
        if self.ok:
            return
        detail = "; ".join(
            f"{name}: {error}" for name, error in sorted(self.failures.items())
        )
        raise GateError(f"{what} failed: {detail}")


class Group:
    """Runs steps, honouring what they contend for, and reports every failure."""

    def __init__(self, name: str, steps: list[Step], *, workers: int | None = None) -> None:
        self.name = name
        self._steps = steps
        self._workers = workers
        _require_no_shared_exclusive(steps)

    def run(self) -> Outcome:
        """Concurrent steps first, then the serial ones, one at a time.

        Order matters between the two halves rather than within them: a serial
        step usually contends for something the concurrent ones were producing.
        """
        outcome = Outcome()
        concurrent = [step for step in self._steps if not step.is_serial]
        serial = [step for step in self._steps if step.is_serial]

        if concurrent:
            workers = self._workers or len(concurrent)
            with ThreadPoolExecutor(max_workers=workers) as pool:
                futures = {step: pool.submit(step.run) for step in concurrent}
            for step, future in futures.items():
                _record(outcome, step, future.exception())

        for step in serial:
            try:
                step.run()
                outcome.ran.append(step.name)
            except Exception as error:
                # Broad on purpose: the failure is recorded against the step's
                # name and reported with every other, not raised here.
                _record(outcome, step, error)

        return outcome


def _record(outcome: Outcome, step: Step, error: BaseException | None) -> None:
    outcome.ran.append(step.name)
    if error is not None:
        outcome.failures[step.name] = error


def _require_no_shared_exclusive(steps: list[Step]) -> None:
    """Two concurrent claimants of one exclusive resource is a wiring bug.

    Caught when the group is built rather than when the two steps happen to
    overlap, because overlapping is a scheduling accident and this is not.
    """
    concurrent = [step for step in steps if not step.is_serial]
    seen: dict[str, str] = {}
    for step in concurrent:
        for resource in step.contends:
            if resource.name in seen:
                raise GateError(
                    f"{step.name} and {seen[resource.name]} both contend for "
                    f"{resource.name} but neither is serial: {resource.reason}"
                )
            seen[resource.name] = step.name

    for step in steps:
        if step.contends and not step.is_serial:  # pragma: no cover - defensive
            raise GateError(f"{step.name} declares contention but is not serial")
