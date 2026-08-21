"""Which runner an action that leaves the kernel sandbox is given.

Its own module because both `actions` and `outside` need it and each is
already at the 300-line ceiling this package enforces on itself, and because
importing it from either of those would close a cycle.
"""

from __future__ import annotations

from .context import Context
from .errors import GateError
from .proc import Runner


def escaping_runner(context: Context, what: str) -> Runner:
    """The outside runner, for an action that declared it must escape.

    Refuses rather than falling back on the sandboxed runner. That fallback is
    what made `outside_sandbox=True` silently mean nothing under a command
    holding no `Egress`: the action ran inside the sandbox and failed hours
    later on something that named neither. See
    `tests/citadel/test_escaping_steps_have_an_egress.py`.
    """
    if context.outside_runner is None:
        raise GateError(
            f"{what} must run outside the kernel sandbox, but this command holds "
            "no egress to run it with: declare `outside_egress` and include "
            "`Egress` in its resources."
        )
    return context.outside_runner
