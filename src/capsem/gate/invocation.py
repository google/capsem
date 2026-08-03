"""One invocation, and how it is allowed to be written down.

Split from `proc`, which performs them. The seam is between what a command
*is* and what running one *does*, and it exists because a third thing lives
here now: which parts of an invocation are a credential.

`crosscompile` read the checkout's Tauri private key and password and put both
into `docker run`'s argv. From argv they reached the process listing -- which
every user on the machine can read, and which no amount of log filtering
covers -- and then the run log, the error text of the failed build, `run.end`,
and the summary. `runlog` promises a run directory is safe to attach to a bug
report; nothing enforced it from this side.

So secrecy is declared on the command by whoever knows the value is a
credential, and there is no rendering that ignores the declaration. `execute`
reads the real values. Everything else -- `__str__`, the journal, the failure
text -- goes through `evidence_argv` and `evidence_env`.
"""

from __future__ import annotations

import shlex
from dataclasses import dataclass, field
from pathlib import Path

#: What a secret renders as. One spelling, so a reader who greps a run
#: directory for it finds every place a value was withheld.
REDACTED = "<redacted>"


@dataclass(frozen=True)
class Command:
    """One invocation, in the form the runner will execute it."""

    argv: tuple[str, ...]
    cwd: Path | None = None
    env: dict[str, str] = field(default_factory=dict)
    """Additions to the inherited environment, not a replacement for it."""
    capture: bool = False
    check: bool = True
    log: Path | None = None
    """Append combined output here instead of streaming it.

    Two build lanes streaming to one terminal interleave into something nobody
    can read, so each concurrent lane writes its own log and only a failing
    lane's tail is surfaced.
    """

    secret_env: frozenset[str] = frozenset()
    """Names in `env` whose values must never be rendered anywhere.

    Secrecy is a property of the invocation, declared once by whoever knows
    the value is a credential -- not a list of key names some renderer
    remembers to filter. `execute` is the only reader of the real values; every
    other path below goes through `evidence_env` and `evidence_argv`.
    """

    @property
    def evidence_env(self) -> dict[str, str]:
        """The environment as it may be written down. Names kept, values not.

        The name is the part worth having: "which variable was set" is what a
        reader needs, and it is not the part that is a credential.
        """
        return {
            name: REDACTED if name in self.secret_env else value
            for name, value in self.env.items()
        }

    @property
    def evidence_argv(self) -> tuple[str, ...]:
        """Argv with any secret *value* removed, wherever it appears.

        Belt and braces: the rail that leaked spelled `-e NAME=value` into
        argv. Declaring the name makes that spelling safe too, so a call site
        that reintroduces it fails closed rather than quietly.
        """
        values = [self.env[name] for name in self.secret_env if self.env.get(name)]
        if not values:
            return self.argv
        rendered = []
        for part in self.argv:
            for value in values:
                part = part.replace(value, REDACTED)
            rendered.append(part)
        return tuple(rendered)

    def __str__(self) -> str:
        """Always the safe rendering.

        Not a separate `safe_render()` beside a raw `__str__`: every failure
        path, every log line and every `repr` in a traceback reaches for
        `str`, and a version of this that leaks is a version something will
        eventually call.
        """
        assignments = " ".join(
            f"{name}={value if name in self.secret_env else shlex.quote(value)}"
            for name, value in sorted(self.evidence_env.items())
        )
        return f"{assignments} {shlex.join(self.evidence_argv)}".strip()
