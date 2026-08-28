"""Whether two steps could ever be running at the same time.

One predicate, because two copies of this rule is exactly how the duplicate-
writer guard came to agree with the bug it was written to catch: the validator
intersected contention *names* and the test reimplemented the same
intersection, so two producers both holding one resource `shared` -- a
readers-lock, designed to overlap -- passed as "serialized" on both sides.

The scheduler asks the same question when it decides what may start, so a
disagreement between what validation blesses and what execution permits is not
representable either.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:  # pragma: no cover - imported for typing only
    from .execution import Step
    from .harnessschema import Exclusive


def can_overlap(first: Step, second: Step) -> bool:
    """Whether these two may be in flight together.

    They may not when they claim one resource and at least one of them claims
    it exclusively. Two shared claims admit each other -- that is what shared
    means, and the asset lanes depend on it.
    """
    held = {resource.name: resource.shared for resource in first.contends}
    return not any(
        resource.name in held and not (held[resource.name] and resource.shared)
        for resource in second.contends
    )


class Claims:
    """What is held right now, and whether one more step may start.

    Owned by the coordinator rather than by the workers. Every ready step used
    to be submitted and each worker then blocked *inside* the resource lock
    until its claim came free -- so the pool had to be as large as the plan
    (eighty-one threads for the candidate gate), because any smaller bound
    could fill every worker with steps that were only waiting while the one
    that could actually run had nowhere to go.
    """

    def __init__(self) -> None:
        self._shared: dict[str, int] = {}
        self._exclusive: set[str] = set()

    def compatible(self, step: Step) -> bool:
        """Whether this step's claims are all currently free enough."""
        for resource in step.contends:
            if resource.name in self._exclusive:
                return False
            if not resource.shared and self._shared.get(resource.name):
                return False
        return True

    def reserve(self, step: Step) -> None:
        for resource in step.contends:
            self._hold(resource, +1)

    def release(self, step: Step) -> None:
        for resource in step.contends:
            self._hold(resource, -1)

    def _hold(self, resource: Exclusive, delta: int) -> None:
        if resource.shared:
            count = self._shared.get(resource.name, 0) + delta
            if count:
                self._shared[resource.name] = count
            else:
                self._shared.pop(resource.name, None)
        elif delta > 0:
            self._exclusive.add(resource.name)
        else:
            self._exclusive.discard(resource.name)
