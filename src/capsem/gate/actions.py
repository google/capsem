"""The smallest reusable units of gate work: things that run a program.

A step used to be a callable, and a callable is opaque in three ways that
matter. It cannot say what it would do, so a dry run can only list step names.
It cannot be timed below itself, so "the gate is slow" stays a forty-minute
mystery rather than resolving to a line. And it shares nothing, so the twelfth
piece of work that needs to copy a tree writes `shutil.copytree` again.

An action answers both questions -- `render` says what it would do, `perform`
does it -- and the two are deliberately independent: `render` must never touch
the machine, because a dry run with side effects is not a dry run.

This module holds the primitives that invoke a program. `fileactions` holds the
ones that manipulate the tree. Both go through `Runner`, which is the single
point every invocation passes through and therefore the only place the run log
has to hook.
"""

from __future__ import annotations

from abc import ABC, abstractmethod
from collections.abc import Callable
from pathlib import Path
from typing import ClassVar

from .context import Context
from .invocation import Command
from .opacity import CallJustification


class Action(ABC):
    """One thing the gate can do, which can also describe itself.

    Subclasses declare their name at class definition:

        class Run(Action, name="run"): ...

    A required class keyword rather than a defaulted attribute, for the same
    reason `Resource` requires one: the name is what the run log records, and
    a defaulted one turns a timing report into a column of `action`.
    """

    name: ClassVar[str]

    def __init_subclass__(cls, *, name: str, **kwargs: object) -> None:
        super().__init_subclass__(**kwargs)
        cls.name = name

    @abstractmethod
    def render(self) -> str:
        """What this would do, in one line, concrete enough to run by hand.

        Must not touch the machine. This is what `--dry-run` prints, and its
        whole value is that reading it is free.
        """

    @abstractmethod
    def perform(self, context: Context) -> None:
        """Do it. Raise `GateError` to fail the owning step."""


class Run(Action, name="run"):
    """Invoke a program, with its arguments kept as separate values.

    Separate values rather than a string, because the moment a path with a
    space reaches a shell fragment it becomes a quoting bug that only appears
    on the machine that has such a path.
    """

    def __init__(
        self,
        argv: list[str] | tuple[str, ...],
        *,
        cwd: Path | None = None,
        env: dict[str, str] | None = None,
        check: bool = True,
        log: Path | None = None,
    ) -> None:
        self._command = Command(
            argv=tuple(str(part) for part in argv),
            cwd=cwd,
            env=dict(env or {}),
            check=check,
            log=log,
        )

    def render(self) -> str:
        """The command, and where it runs if that is not the checkout root.

        Four identical `pnpm install` lines differing only by directory are
        four lines a reader cannot tell apart, which defeats the point of
        printing them.
        """
        rendered = str(self._command)
        if self._command.cwd is None:
            return rendered
        return f"(in {self._command.cwd.name}) {rendered}"

    def perform(self, context: Context) -> None:
        command = self._command
        context.runner.run(
            command.argv,
            cwd=command.cwd,
            # The action's own environment wins: a context sets what a whole
            # scope shares, and the narrower scope is the one that meant it.
            env={**context.env, **command.env},
            check=command.check,
            log=command.log,
        )


class Script(Action, name="script"):
    """Run a checked-in Python script through the project's environment.

    Through `uv`, never a bare `python3`: the interpreter on PATH is whatever
    the machine happens to have, and on a release runner that is not the one
    the lockfile pins.
    """

    def __init__(
        self,
        relative: str,
        *args: object,
        env: dict[str, str] | None = None,
        check: bool = True,
    ) -> None:
        self._relative = relative
        self._args = tuple(str(arg) for arg in args)
        self._env = dict(env or {})
        self._check = check

    def render(self) -> str:
        """The relative path, because the checkout it sits in is implied."""
        return str(
            Command(
                argv=("uv", "run", "python", self._relative, *self._args),
                env=self._env,
            )
        )

    def perform(self, context: Context) -> None:
        context.runner.script(
            self._relative,
            *self._args,
            env={**context.env, **self._env},
            check=self._check,
        )


class Shell(Action, name="shell"):
    """A fragment where the shell itself is the point -- pipes, globs, `&&`.

    Reach for this only when that is true. A fragment that merely spells out a
    command belongs in `Run`, where its arguments stay separate values instead
    of becoming a quoting problem, and where the dry run can show them.
    """

    def __init__(
        self,
        fragment: str,
        *,
        cwd: Path | None = None,
        env: dict[str, str] | None = None,
        check: bool = True,
    ) -> None:
        self._fragment = fragment
        self._cwd = cwd
        self._env = dict(env or {})
        self._check = check

    def render(self) -> str:
        return self._fragment

    def perform(self, context: Context) -> None:
        context.runner.bash(
            self._fragment,
            cwd=self._cwd,
            env={**context.env, **self._env},
            check=self._check,
        )


class Call(Action, name="call"):
    """Work that is not expressed as primitives yet.

    A bridge, and deliberately a poor one: a dry run can only print the
    description it was handed, so a plan built from these says far less than a
    plan built from `Run` and `Copy`. That is the incentive, not an oversight.

    `why` is required because the docstring here used to claim every remaining
    instance carried signing material -- true of exactly one of twenty, and a
    rationale that covers everything stops being a reason for anything.
    """

    def __init__(
        self,
        description: str,
        action: Callable[[Context], None],
        *,
        justification: CallJustification,
    ) -> None:
        self._description = description
        self._action = action
        self.justification = justification

    def render(self) -> str:
        """The description, and what kind of opacity this is.

        The kind is in the dry run because that is where a reader decides
        whether the plan tells them enough -- and `pure-inspection` beside a
        line is the signal that it could have told them more.
        """
        return f"{self._description} [{self.justification.kind.value}]"

    def perform(self, context: Context) -> None:
        self._action(context)


class Launch(Action, name="launch"):
    """Start a process that outlives the step, and record its pid.

    Distinct from `Run`, which waits. A daemon the gate starts has to be
    stopped by whatever is holding it, which is why the pid goes to a file the
    `pidfiles` module can read rather than being remembered.
    """

    def __init__(
        self,
        argv: list[str] | tuple[str, ...],
        *,
        pidfile: Path,
        env: dict[str, str] | None = None,
    ) -> None:
        self._argv = tuple(str(part) for part in argv)
        self._pidfile = pidfile
        self._env = dict(env or {})

    def render(self) -> str:
        return f"start (detached) {Command(argv=self._argv, env=self._env)}"

    def perform(self, context: Context) -> None:
        pid = context.runner.launch(self._argv, env={**context.env, **self._env})
        self._pidfile.parent.mkdir(parents=True, exist_ok=True)
        self._pidfile.write_text(str(pid), encoding="utf-8")
        context.journal.note(f"{self._argv[0]} started (pid {pid})")
