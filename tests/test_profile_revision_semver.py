"""Profile revisions are semver, for first-party and corp-authored profiles alike.

A profile revision is its tag: what a corp operator reads, what asset reuse is
keyed on, and what immutable publication is enforced against. Profiles are
orthogonal, so each carries its own independent version -- `code` moving says
nothing about `co-work` -- and that version is a separate axis from the
`min_capsem_version`/`max_capsem_version` window the profile declares against
the Capsem binary.

The scheme this replaces was a date plus a counter (`2026.06.08.9`). It could
not order releases: the date recorded when someone last edited the field, not
when the assets were built, so a July build shipped wearing a June date; the
counter counted hand-edits rather than publications, so `.8` and `.9` existed
having never been released. Nothing rejected either, because nothing checked.
"""

from __future__ import annotations

import re
import tomllib
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent
PROFILES_DIR = PROJECT_ROOT / "config" / "profiles"

# Strict semver: MAJOR.MINOR.PATCH, no leading zeroes, optional prerelease and
# build metadata. Deliberately not a loose "digits and dots" pattern -- that is
# what let a four-component date through.
SEMVER = re.compile(
    r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)


def _profiles() -> dict[str, dict]:
    found = {}
    for profile_toml in sorted(PROFILES_DIR.glob("*/profile.toml")):
        found[profile_toml.parent.name] = tomllib.loads(
            profile_toml.read_text(encoding="utf-8")
        )
    assert found, f"no profiles found under {PROFILES_DIR}"
    return found


def test_every_profile_revision_is_semver() -> None:
    offenders = {
        name: profile.get("revision")
        for name, profile in _profiles().items()
        if not SEMVER.match(str(profile.get("revision", "")))
    }

    assert not offenders, (
        "profile revisions must be semver MAJOR.MINOR.PATCH so releases order "
        "and compatibility windows mean something: "
        + ", ".join(f"{name}={rev!r}" for name, rev in sorted(offenders.items()))
    )


def test_profile_revisions_are_independent_per_profile() -> None:
    """Nothing may require two profiles to share a revision.

    Profiles are orthogonal. This does not demand they differ -- two profiles
    may legitimately sit at the same version -- only that the schema carries
    one revision per profile rather than a single global one.
    """
    profiles = _profiles()

    for name, profile in profiles.items():
        assert "revision" in profile, f"profile {name} declares no revision of its own"


def test_compatibility_window_is_semver_when_declared() -> None:
    """`min_capsem_version`/`max_capsem_version` bound the binary, not the profile.

    They are a different axis from the profile's own revision, and they are
    compared with semver ordering by capsem-admin, so a non-semver bound would
    be rejected at release time rather than here.
    """
    for name, profile in _profiles().items():
        for field in ("min_capsem_version", "max_capsem_version"):
            bound = profile.get(field)
            if bound is None:
                continue
            assert SEMVER.match(str(bound)), (
                f"profile {name} declares {field}={bound!r}, which is not semver "
                "and cannot be ordered against a Capsem release"
            )


def test_capsem_version_patch_is_not_a_timestamp() -> None:
    """The Capsem binary version must order releases, not record an instant.

    `1.6.1785421421` parses as semver but its patch is a Unix timestamp, so a
    compatibility window can only ever express "built before/after this
    moment". Two releases a second apart look as far apart as two a year
    apart, and the patch communicates nothing to the operator writing a
    `min_capsem_version`.
    """
    workspace = tomllib.loads((PROJECT_ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    version = workspace["workspace"]["package"]["version"]

    assert SEMVER.match(version), f"workspace version is not semver: {version!r}"
    patch = int(version.split("+")[0].split("-")[0].split(".")[2])
    assert patch < 1_000_000, (
        f"workspace version {version!r} carries a timestamp patch ({patch}); "
        "patches must increment so releases can be ordered and ranged"
    )


def test_internal_crate_deps_do_not_pin_a_version() -> None:
    """Sibling crates are referenced by path, never by a pinned version.

    A pinned internal version is a second place the workspace version lives,
    and it drifts silently: `capsem-guard = { version = "1.0.1776688771" }`
    sat unnoticed for months because caret matching accepted every 1.x, then
    broke the entire workspace build the moment the line moved to 0.6.
    """
    offenders = []
    for cargo_toml in sorted((PROJECT_ROOT / "crates").glob("*/Cargo.toml")):
        manifest = tomllib.loads(cargo_toml.read_text(encoding="utf-8"))
        for section in ("dependencies", "dev-dependencies", "build-dependencies"):
            for name, spec in (manifest.get(section) or {}).items():
                if not name.startswith("capsem"):
                    continue
                if isinstance(spec, dict) and "version" in spec:
                    offenders.append(
                        f"{cargo_toml.parent.name}/{section}: {name} pins {spec['version']!r}"
                    )

    assert not offenders, (
        "internal crate dependencies must be path-only so the workspace version "
        "lives in exactly one place:\n  " + "\n  ".join(offenders)
    )


def test_release_skill_documents_semver_discipline() -> None:
    """The rule an operator reads must match the rule capsem-admin enforces.

    Corp operators author profiles without touching this repository's code, so
    the skill is where they meet the requirement. If it drifts from the
    enforcement, they learn the rule from a rejected release instead.
    """
    skill = (PROJECT_ROOT / "skills" / "release-process" / "SKILL.md").read_text(
        encoding="utf-8"
    )

    for required in (
        "parse_profile_revision",
        "ensure_revision_advances",
        "min_capsem_version",
        "profiles-<hash>",
    ):
        assert required in skill, f"release skill must document {required!r}"

    assert "semver" in skill.lower(), "release skill must name the versioning scheme"
