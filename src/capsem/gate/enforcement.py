"""Which commands may only run under kernel enforcement.

One predicate, in its own file because both plausible homes -- `command` and
`sandbox` -- are already at the boundary guard's ceiling, and a rule about who
must be sandboxed is not really a detail of either.
"""

from __future__ import annotations

from .qualificationevidence import QualificationPolicy


def enforcement_required(command) -> bool:
    """Whether this command may only run under kernel enforcement.

    Publishing is the reason, not journal consumption. Keyed on the
    qualification policy alone, a channel that consumes no operator journal --
    nightly -- could publish from a permissive sandbox, which is backwards: it
    has less human scrutiny than stable, not more.
    """
    return (
        command.complete_qualification
        or command.publishes
        or command.qualification_policy is QualificationPolicy.REQUIRE
    )
