"""What this gate run is proving, and against which bytes.

A release lane does not run a different gate. It runs the same one against
artifacts a manifest already selected, so what ships is what was proved rather
than something rebuilt beside it. Which family arrives pulled is the only
difference, and it is decided by environment variables the workflow exports.

Three modules used to decide that independently -- `module_artifacts` from the
input directory, `module_functional` from the same one, `module_glowup` from
the package path -- and nothing compared their answers. Every partial
combination therefore built a plausible plan:

    input directory only  ->  pulled assets, and a *locally rebuilt* package
    package only          ->  a pulled package, and *locally rebuilt* assets

Both are green, both take an hour, and both prove source bytes in place of the
manifest-selected ones. One dropped `GITHUB_ENV` line is all it takes, and the
result looks exactly like a passing release.

So the state is one indivisible value with three legal shapes, parsed once and
passed down. There is no boolean `pulled=True` anywhere below this file: the
paths themselves travel, because a lane that knows it is a release lane but not
which package it was handed is the same defect one level along.
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from typing import TYPE_CHECKING

from .errors import GateError

if TYPE_CHECKING:  # pragma: no cover - imported for typing only
    from collections.abc import Mapping

    from .config import GateConfig


class Mode(Enum):
    """The three states a gate run can legally be in."""

    LOCAL = "local"
    """Nothing was handed over; both families are built from this checkout."""

    BINARY_RELEASE = "binary-release"
    """The packages are the candidate; every profile arrives by digest."""

    PROFILE_RELEASE = "profile-release"
    """One profile is the candidate; the package arrives by digest."""


@dataclass(frozen=True, slots=True)
class Qualification:
    """The complete release state, or its absence. Never partially one."""

    mode: Mode
    bin_dir: str
    """Where the binaries under test live. Its fallback is `[modules]`'s, not
    a second spelling of the same path in Python."""

    input_dir: str | None = None
    package: str | None = None
    profile: str | None = None

    @property
    def pulled(self) -> bool:
        """Whether a manifest already chose the artifacts to prove."""
        return self.mode is not Mode.LOCAL

    @classmethod
    def from_environment(
        cls, config: GateConfig, environ: Mapping[str, str] | None = None
    ) -> Qualification:
        """Read the state once, and refuse anything that is not one of three.

        `environ` defaults to the process environment; tests pass their own
        rather than mutating it, which is also what lets the table below be a
        table rather than eight `monkeypatch` fixtures.
        """
        import os

        source = os.environ if environ is None else environ
        settings = config.modules

        def present(name: str) -> str | None:
            # An exported-but-empty variable is absent. `echo "VAR=" >>
            # $GITHUB_ENV` is one deleted shell expansion away, and treating
            # the result as present sends the lane looking for a directory
            # named "".
            value = (source.get(name) or "").strip()
            return value or None

        names = (settings.release_input_dir, settings.release_package, settings.release_profile)
        input_dir, package, profile = (present(name) for name in names)

        if input_dir and package:
            mode = Mode.PROFILE_RELEASE if profile else Mode.BINARY_RELEASE
        elif not any((input_dir, package, profile)):
            mode = Mode.LOCAL
        else:
            raise _refusal(names, (input_dir, package, profile))

        return cls(
            mode=mode,
            input_dir=input_dir,
            package=package,
            profile=profile,
            bin_dir=present(settings.release_bin_dir) or settings.default_bin_dir,
        )


def _refusal(names: tuple[str, ...], values: tuple[str | None, ...]) -> GateError:
    """Name both sides: what is set, and what has to join it.

    Whoever reads this is looking at a workflow log after a release stopped,
    and the useful sentence is which line to add -- not that something is
    inconsistent.
    """
    set_names = [name for name, value in zip(names, values, strict=True) if value]
    missing = [name for name, value in zip(names, values, strict=True) if not value]
    return GateError(
        "this is a partial release environment, and a gate cannot prove half a "
        f"release: {', '.join(set_names)} "
        f"{'is' if len(set_names) == 1 else 'are'} set while "
        f"{', '.join(missing)} {'is' if len(missing) == 1 else 'are'} not. "
        "A run either builds both artifact families locally (none of these "
        f"set), proves a binary candidate ({names[0]} and {names[1]}), or "
        f"proves one profile ({names[0]}, {names[1]} and {names[2]}). "
        "Anything else verifies manifest-selected bytes in one family and "
        "source-built bytes in the other, which is not the release that ships."
    )
