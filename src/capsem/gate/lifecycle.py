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
from typing import ClassVar

from .errors import GateError


class Resource(ABC):
    """Something acquired for the length of a phase and then given back.

    Subclasses implement `acquire` and `release`, and declare their name at
    class definition:

        class Workspace(Resource, name="workspace"): ...

    A required class keyword rather than a defaulted attribute, because the
    name is what a teardown failure says failed. Defaulted, forgetting it
    produces `failed to release: resource: ...` at the end of a forty-minute
    run; required, it is a `TypeError` at import.

    `preserve` is optional and exists for resources holding diagnostics worth
    copying out before teardown removes them.
    """

    #: Named in failure messages, so a teardown error says what failed.
    name: ClassVar[str]

    def __init_subclass__(cls, *, name: str, **kwargs: object) -> None:
        super().__init_subclass__(**kwargs)
        cls.name = name

    @abstractmethod
    def acquire(self) -> None:
        """Take the resource. Raising here means nothing was taken."""

    @abstractmethod
    def release(self) -> None:
        """Give it back. Runs on every path, including an aborted one."""

    def environment(self) -> dict[str, str]:
        """What every command inside this resource's scope inherits.

        The reason isolation is structural rather than remembered. A workspace
        exports `CAPSEM_HOME` here, once, and `GateCommand.execute` folds it
        into the context -- so acquiring the workspace is what makes an action
        run inside it. Previously `Workspace.environment` existed and nothing
        production ever read it, which meant every command advertised as
        isolated was running against the developer's own `~/.capsem`.

        Empty for the resources that guard something rather than relocate it.
        """
        return {}

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
    failure: BaseException | None = None
    try:
        for resource in resources:
            resource.acquire()
            acquired.append(resource)
        # What was taken, not what was asked for. The two differ only when an
        # acquire raised -- and then the body never runs -- but the body reads
        # this to build its environment, and a resource that is not there must
        # not be telling commands where to run.
        yield tuple(acquired)
    except BaseException as error:
        failure = error
        _preserve(acquired, error)
        raise
    finally:
        # A `finally` that raises *replaces* the exception in flight, so a
        # teardown failure used to be the only thing the operator was told:
        # they read that a process leaked, and never learned which test failed
        # and caused the leak. Cleanup failures are reported here and attached
        # to the primary error, and only become the error themselves when
        # there is no primary one to lose.
        _release(acquired, primary=failure)


def environment_of(resources: tuple[Resource, ...]) -> dict[str, str]:
    """What the acquired resources export, later ones winning.

    Acquisition order is precedence order, the way a stack gives it: a service
    acquired inside a workspace may narrow one of the workspace's variables,
    and the inner scope is the one that meant it.
    """
    environment: dict[str, str] = {}
    for resource in resources:
        environment.update(resource.environment())
    return environment


def _preserve(acquired: list[Resource], error: BaseException) -> None:
    """Evidence first, in reverse, and never at the expense of the failure."""
    for resource in reversed(acquired):
        try:
            resource.preserve(error)
        except Exception as failure:
            # Broad on purpose -- see the method docstring: evidence collection
            # must never replace the failure the operator needs to read.
            print(f"failed to preserve {resource.name} evidence: {failure}")


def _release(acquired: list[Resource], *, primary: BaseException | None = None) -> None:
    """Release everything, then report anything that would not let go.

    One resource refusing to release must not strand the others, so every
    release is attempted before any failure is raised.

    When something already failed, that failure is the one the operator needs;
    a teardown error raised from here would silently replace it. The cleanup
    failures are still surfaced -- printed, and added to the primary error's
    notes so a traceback carries them -- but the primary error is what
    propagates.
    """
    failures: list[str] = []
    for resource in reversed(acquired):
        try:
            resource.release()
        except Exception as failure:
            # Broad on purpose: every release is attempted, and the failures
            # are aggregated below rather than stopping at the first.
            failures.append(f"{resource.name}: {failure}")
    if not failures:
        return

    reported = "failed to release: " + "; ".join(failures)
    if primary is None:
        raise GateError(reported)
    print(reported)
    primary.add_note(reported)
