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

So the state is a discriminated union with three legal shapes, and the illegal
ones are unrepresentable rather than merely unreachable: a dataclass with four
optional fields let `LOCAL` carry an input directory perfectly happily, and
kept the invariant inside one parsing function instead of in the type. There is
no boolean `pulled=True` anywhere below this file either -- the exact paths
travel, because a lane that knows it is a release lane but not which package it
was handed is the same defect one level along.
"""

from __future__ import annotations

from enum import StrEnum
from typing import TYPE_CHECKING, Annotated, Literal

from pydantic import Field, StringConstraints, TypeAdapter

from .configschema import Strict
from .errors import GateError

if TYPE_CHECKING:  # pragma: no cover - imported for typing only
    from collections.abc import Mapping

    from .config import GateConfig

#: Path text a workflow handed over. Validated as *text* -- non-empty, no
#: stray whitespace -- and never against the filesystem: whether it exists is a
#: question for a plan step, and a `--dry-run` that stats the disk is a dry run
#: that depends on the machine it is only describing.
GatePath = Annotated[str, StringConstraints(min_length=1, strip_whitespace=True)]

#: A profile directory name. The grammar only; whether this checkout *has* one
#: is a catalog question that `profiles.selected` already owns.
ProfileName = Annotated[str, StringConstraints(pattern=r"^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$")]


class Mode(StrEnum):
    """The three states a gate run can legally be in."""

    LOCAL = "local"
    """Nothing was handed over; both families are built from this checkout."""

    BINARY_RELEASE = "binary-release"
    """The packages are the candidate; every profile arrives by digest."""

    PROFILE_RELEASE = "profile-release"
    """One profile is the candidate; the package arrives by digest."""


class LocalQualification(Strict):
    """Nothing was handed over. It has nowhere to put a release input."""

    mode: Literal[Mode.LOCAL] = Mode.LOCAL
    bin_dir: GatePath

    # Declared as fields typed `None` rather than left out. Absent, they would
    # be attributes a consumer could not read; typed `None`, constructing a
    # local state that carries an input directory is a validation error rather
    # than an ordinary object the parser happens never to build.
    input_dir: None = None
    package: None = None
    profile: None = None

    @property
    def pulled(self) -> bool:
        return False


class BinaryQualification(Strict):
    """The packages are the candidate; every profile arrives by digest."""

    mode: Literal[Mode.BINARY_RELEASE] = Mode.BINARY_RELEASE
    input_dir: GatePath
    package: GatePath
    bin_dir: GatePath

    profile: None = None
    """A binary lane resolves every profile the manifest names, so there is no
    single one to boot. A field rather than a property, so the subclass below
    can narrow it -- Pydantic refuses to shadow a parent's property."""

    @property
    def pulled(self) -> bool:
        return True


class ProfileQualification(BinaryQualification):
    """One profile is the candidate; the package arrives by digest."""

    mode: Literal[Mode.PROFILE_RELEASE] = Mode.PROFILE_RELEASE  # type: ignore[assignment]
    profile: ProfileName  # type: ignore[assignment]


Qualification = Annotated[
    LocalQualification | BinaryQualification | ProfileQualification,
    Field(discriminator="mode"),
]

_QUALIFICATION = TypeAdapter(Qualification)


def from_environment(
    config: GateConfig, environ: Mapping[str, str] | None = None
) -> LocalQualification | BinaryQualification | ProfileQualification:
    """Read the state once, and refuse anything that is not one of three.

    `environ` defaults to the process environment; tests pass their own rather
    than mutating it, which is also what lets the combination table be a table
    rather than eight `monkeypatch` fixtures.
    """
    import os

    source = os.environ if environ is None else environ
    settings = config.modules

    def present(name: str) -> str | None:
        # An exported-but-empty variable is absent. `echo "VAR=" >> $GITHUB_ENV`
        # is one deleted shell expansion away, and treating the result as
        # present sends the lane looking for a directory named "".
        value = (source.get(name) or "").strip()
        return value or None

    names = (settings.release_input_dir, settings.release_package, settings.release_profile)
    input_dir, package, profile = (present(name) for name in names)
    bin_dir = present(settings.release_bin_dir) or settings.default_bin_dir

    if input_dir and package:
        shape: dict[str, object] = {
            "mode": Mode.PROFILE_RELEASE if profile else Mode.BINARY_RELEASE,
            "input_dir": input_dir,
            "package": package,
            "bin_dir": bin_dir,
        }
        if profile:
            shape["profile"] = profile
    elif not any((input_dir, package, profile)):
        shape = {"mode": Mode.LOCAL, "bin_dir": bin_dir}
    else:
        raise _refusal(names, (input_dir, package, profile))

    return _QUALIFICATION.validate_python(shape)


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
