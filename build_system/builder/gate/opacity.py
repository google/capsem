"""Why a piece of work is opaque to a dry run, as an answer it has to give.

`Call` renders as prose, so every one of them is a plan saying less than it
could. Twenty of them shared a single rationale -- "a package build carries
signing material" -- which is true of exactly one, and a reason that covers
everything stops being a reason for anything.

Its own module rather than a corner of `actions`, because it is not a
primitive: it is the vocabulary for admitting that something is not one yet.
Two of the four kinds are an invitation to stop being a `Call` at all.
"""

from __future__ import annotations

from enum import StrEnum
from typing import Annotated

from pydantic import StringConstraints, model_validator

from .configschema import Strict


class OpaqueKind(StrEnum):
    """Why a piece of work is not an ordinary declared action.

    A closed vocabulary, because these are Capsem's own categories rather than
    a third-party tool's. Two of the four are an invitation to stop being a
    `Call` at all.
    """

    RUNTIME_DERIVED = "runtime-derived"
    """Its argv is only known once the step is running: what to reclaim, which
    profiles exist, which package the builder just wrote."""

    SECRET_BEARING = "secret-bearing"
    """Its environment carries a credential a dry run must not print. Exactly
    one phase: the package build and its Tauri signing key."""

    DOMAIN_TRANSACTION = "domain-transaction"
    """Several operations that only make sense together, whose own primitives
    are already journaled -- an install, an asset assembly."""

    PURE_INSPECTION = "pure-inspection"
    """It decides or reports and starts no subprocess of its own. The weakest
    reason, and named to look weak: work like this can usually be a declared
    action with its own render and its own timing. A `Call` here is a to-do,
    not a design."""


class Effect(StrEnum):
    """The closed set of machine surfaces an opaque call may touch."""

    PROCESS = "process"
    FILESYSTEM = "filesystem"
    NETWORK = "network"
    HOST_STATE = "host-state"


def machine_effects(*items: Effect) -> frozenset[Effect]:
    """Build an effect set through a Ty-enforced closed-vocabulary seam."""
    if any(not isinstance(item, Effect) for item in items):
        raise TypeError("machine_effects accepts only Effect enum members")
    return frozenset(items)


class CallJustification(Strict):
    """The answer a `Call` has to give: which kind, why, and what it affects.

    The docstring here used to carry one rationale for all twenty instances --
    "a package build carries signing material" -- which is true of exactly one
    of them. A reason that covers everything stops being a reason, so each one
    states its own and a contract reads them back.
    """

    kind: OpaqueKind
    reason: Annotated[str, StringConstraints(min_length=20, strip_whitespace=True)]
    effects: frozenset[Effect] = frozenset()

    @model_validator(mode="after")
    def _inspection_touches_nothing(self) -> CallJustification:
        """`PURE_INSPECTION` that writes is not inspection.

        The label is the weakest of the four and therefore the most tempting;
        letting it declare a filesystem effect would make it a synonym for
        "opaque" and lose the distinction entirely.
        """
        if self.kind is OpaqueKind.PURE_INSPECTION and self.effects - {Effect.PROCESS}:
            raise ValueError(
                "a pure inspection may not declare "
                f"{sorted(self.effects - {Effect.PROCESS})}; "
                "it decides or reports, and anything else is a transaction"
            )
        return self
