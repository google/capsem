"""Acquiring things the gate must give back, and giving them back in order.

Every gate module holds resources that outlive a single call: a privileged
container, a manifest handoff the installer will read, an isolated `CAPSEM_HOME`
with a daemon in it, a temporary run directory. Each was written with its own
`try`/`finally`, which is correct until two of them interact -- and they do:

  the handoff must be cleared *before* the container goes, or the next install
  in this checkout inherits a request pointing at a graph that no longer exists

  the service must be stopped *before* the run directory is deleted, because
  stopping it is what flushes `serial.log` -- the file a boot failure is
  argued from

Both are release-order rules, and a `finally` block enforces order only by
where its lines happen to sit. `held` makes it structural: resources are
acquired in order and released in reverse, the way a stack unwinds.

`preserve` is the third phase, and the one an ad-hoc `finally` usually forgets.
It runs only on failure, and before release -- because release is what destroys
the evidence.
"""

from __future__ import annotations

from abc import ABC, abstractmethod
from collections.abc import Iterator
from contextlib import contextmanager

from .errors import GateError


class Resource(ABC):
    """Something acquired for the length of a phase and then given back.

    Subclasses implement `acquire` and `release`; `preserve` is optional and
    exists for resources that hold diagnostics worth copying out before
    teardown removes them.
    """

    #: Named in failure messages, so a teardown error says what failed.
    name: str = "resource"

    @abstractmethod
    def acquire(self) -> None:
        """Take the resource. Raising here means nothing was taken."""

    @abstractmethod
    def release(self) -> None:
        """Give it back. Runs on every path, including an aborted one."""

    def preserve(self, error: BaseException) -> None:  # noqa: B027
        """Copy out anything `release` is about to destroy.

        Deliberately concrete and deliberately empty: most resources have no
        evidence to save, and forcing every one of them to write an empty
        override would be ceremony that teaches readers to skip the method.

        Only on failure, and only before release. Anything raised here is
        reported and swallowed: evidence collection must never replace the
        failure the operator actually needs to read.
        """


@contextmanager
def held(*resources: Resource) -> Iterator[tuple[Resource, ...]]:
    """Acquire in order, release in reverse, preserve on failure first.

    A resource that fails to acquire is not released, and the ones already
    acquired are -- so a half-built phase leaves nothing behind.
    """
    acquired: list[Resource] = []
    try:
        for resource in resources:
            resource.acquire()
            acquired.append(resource)
        yield tuple(resources)
    except BaseException as error:
        _preserve(acquired, error)
        raise
    finally:
        _release(acquired)


def _preserve(acquired: list[Resource], error: BaseException) -> None:
    """Evidence first, in reverse, and never at the expense of the failure."""
    for resource in reversed(acquired):
        try:
            resource.preserve(error)
        except Exception as failure:
            # Broad on purpose -- see the method docstring: evidence collection
            # must never replace the failure the operator needs to read.
            print(f"failed to preserve {resource.name} evidence: {failure}")


def _release(acquired: list[Resource]) -> None:
    """Release everything, then report anything that would not let go.

    One resource refusing to release must not strand the others, so every
    release is attempted before any failure is raised.
    """
    failures: list[str] = []
    for resource in reversed(acquired):
        try:
            resource.release()
        except Exception as failure:
            # Broad on purpose: every release is attempted, and the failures
            # are aggregated below rather than stopping at the first.
            failures.append(f"{resource.name}: {failure}")
    if failures:
        raise GateError("failed to release: " + "; ".join(failures))
