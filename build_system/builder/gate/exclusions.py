"""What a guard is allowed to forgive, and the shape it must be forgiven in.

Every guard eventually meets something it should not fail on, and the shape of
that carve-out decides whether the guard survives contact with a real tree.
Three shapes were tried here in one afternoon and two were wrong:

  - **A count.** "This file has nine discarded verdicts." It fails when
    somebody adds a tenth that is harmless, and passes when somebody changes
    one of the nine into something dangerous. The number is orthogonal to the
    risk.
  - **A name.** "`launchctl` is a cleanup command." It is, at three call sites,
    and it is a real check at a fourth. A program name says nothing about
    whether the verdict mattered.
  - **The exact thing, hashed, with a reason.** This one.

So: an exclusion names its subject exactly, identifies it by a hash of the
*parsed* form rather than the source text, and states why. Hashing the parsed
form is what makes it usable -- requoting a line, reflowing it, or moving it to
another file leaves the hash alone, while changing what the line actually does
changes it, which is a new decision and has to be stated as one.

`reason` has a minimum length because "known", "ok" and "legacy" are not
reasons, and a ledger of those is an exemption list that has learned to spell.

The reconciliation is deliberately symmetric. A ledger that only refuses growth
is an exemption list wearing a ratchet's name: the entry that outlives the
thing it excused is exactly the failure these guards exist to catch, and it is
the one that never announces itself.
"""

from __future__ import annotations

import hashlib
from collections.abc import Iterable
from dataclasses import dataclass
from typing import Annotated, Literal

from pydantic import Field

from .configschema import Strict

#: Long enough that a reason has to be a sentence. Short enough that a real one
#: always clears it. The bar is "someone who finds this in a year knows why".
MINIMUM_REASON = 24

Reason = Annotated[str, Field(min_length=MINIMUM_REASON)]


class Exclusion(Strict):
    """Something a guard knowingly does not fail on.

    `subject` is what is excluded, spelled exactly as the guard reports it, so
    the ledger and the diagnostic are the same string. `reason` says why, and
    is checked for length rather than trusted to exist.
    """

    subject: str
    reason: Reason


class PlatformExclusion(Exclusion):
    """An exclusion that exists only where the underlying action can exist."""

    platforms: tuple[Literal["Darwin", "Linux"], ...] = ()


class HashedExclusion(Exclusion):
    """An exclusion pinned to the exact content it was granted for.

    `digest` is over a canonical form the guard computes -- parsed argv, a
    normalised node, a sorted tuple -- never over raw source. `where` is for
    the human reading the ledger and is not part of the identity, so moving the
    line does not invalidate the decision.
    """

    digest: str
    where: str


def canonical(parts: Iterable[str]) -> str:
    """The identity of a multi-part subject, stable across formatting.

    NUL-joined because it is the one byte that cannot appear in argv, so
    `["a b"]` and `["a", "b"]` cannot collide -- which they do under any
    separator a shell command might legitimately contain.
    """
    return hashlib.sha256("\x00".join(parts).encode("utf-8")).hexdigest()[:12]


@dataclass(frozen=True)
class Reconciliation:
    """What a ledger and reality disagree about."""

    unlisted: tuple[str, ...]
    """Found, and not excused. The guard's actual findings."""

    stale: tuple[str, ...]
    """Excused, and no longer found. The half that keeps this a ledger."""

    @property
    def clean(self) -> bool:
        return not self.unlisted and not self.stale

    def report(self, *, add: str) -> str:
        """A message that says what to do, including the entry to paste in."""
        lines = []
        if self.unlisted:
            lines.append("not excused by the ledger:")
            lines += [f"  {entry}" for entry in self.unlisted]
            lines.append(f"If each is deliberate, add it to {add} with a reason.")
        if self.stale:
            lines.append("excused but no longer present -- remove from the ledger:")
            lines += [f"  {entry}" for entry in self.stale]
        return "\n".join(lines)


def reconcile(found: dict[str, str], excused: Iterable[str]) -> Reconciliation:
    """Compare what a guard found against what the ledger excuses.

    `found` maps each subject's digest to a human-readable rendering, so the
    diagnostic can quote the thing rather than its hash while the identity
    stays exact.
    """
    listed = set(excused)
    return Reconciliation(
        unlisted=tuple(
            f"{digest}  {shown}" for digest, shown in sorted(found.items()) if digest not in listed
        ),
        stale=tuple(sorted(listed - set(found))),
    )
