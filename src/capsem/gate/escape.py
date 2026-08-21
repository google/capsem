"""Which runner an action that leaves the kernel sandbox is given.

Its own module because both `actions` and `outside` need it and each is
already at the 300-line ceiling this package enforces on itself.

The rule is deliberately permissive, and the history is worth keeping. An
earlier version refused when no `Egress` was held, on the theory that a step
declaring `outside_sandbox=True` and running inside one was a silent lie. It
is -- `glowup.package` installs a system package and failed on sudo for
exactly that reason -- but the refusal was wrong far more often than it was
right:

  * A command running with the sandbox off holds a disabled `Egress` by
    construction, so it has no outside runner and nothing to escape.
  * A docker build wants the ordinary environment. Routing it through the
    egress capability runner gives it that runner's deliberately narrow one,
    and the Linux host-builder image failed with empty build arguments.
    Container work belongs to the Docker daemon's own boundary, which
    `AGENTS.md` says must never be replaced by the egress helper.

So: use the outside runner when one is held, and the ordinary runner
otherwise. A step that genuinely needs the egress capability gets it by its
command declaring `outside_egress`, which is what `qualify-binaries` does for
the package install.
"""

from __future__ import annotations

from .context import Context
from .proc import Runner


def escaping_runner(context: Context, what: str) -> Runner:
    """The outside runner when one is held, else the ordinary one."""
    del what
    return context.outside_runner or context.runner
