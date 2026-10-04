"""Every version in the release system is strict semver that orders releases.

The scheme this replaced was a date plus a counter (`2026.06.08.9`). It could
not order releases: the date recorded when someone last edited the field, not
when the assets were built, so a July build shipped wearing a June date; the
counter counted hand-edits rather than publications, so `.8` and `.9` existed
having never been released. Nothing rejected either, because nothing checked.
"""

from __future__ import annotations

import json
import re
import tomllib
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[3]
FIXTURE_GRAPH = (
    PROJECT_ROOT / "tests" / "capsem-release" / "fixtures" / "release-graph-stable-nightly.json"
)

# Strict semver: MAJOR.MINOR.PATCH, no leading zeroes, optional prerelease and
# build metadata. Deliberately not a loose "digits and dots" pattern -- that is
# what let a four-component date through.
SEMVER = re.compile(
    r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)


def _workspace_version() -> str:
    workspace = tomllib.loads((PROJECT_ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    return workspace["workspace"]["package"]["version"]


def test_released_runtime_revision_is_semver_with_a_commit_identity() -> None:
    """`<workspace version>-<first 12 hex of the source commit>` is semver.

    The suffix is a prerelease, so every released commit gets its own runtime
    identity and a nightly re-release at an unchanged workspace version never
    collides with the previous one.
    """
    revision = f"{_workspace_version()}-0123456789ab"

    assert SEMVER.match(revision), revision


def test_compatibility_window_is_semver_when_declared() -> None:
    """`min_capsem_version`/`max_capsem_version` bound the binary, not the runtime.

    They are a different axis from the runtime revision, and capsem-admin
    compares them with semver ordering, so a non-semver bound would be
    rejected at release time rather than here.
    """
    graph = json.loads(FIXTURE_GRAPH.read_text(encoding="utf-8"))
    for channel, manifests in graph["manifests"].items():
        for version, manifest in manifests.items():
            for field in ("min_capsem_version", "max_capsem_version"):
                bound = manifest["runtime"].get(field)
                if bound is None:
                    continue
                assert SEMVER.match(str(bound)), f"{channel} {version} runtime {field}={bound!r}"


def test_capsem_version_patch_is_not_a_timestamp() -> None:
    """The Capsem binary version must order releases, not record an instant.

    `1.6.1785421421` parses as semver but its patch is a Unix timestamp, so a
    compatibility window can only ever express "built before/after this
    moment". Two releases a second apart look as far apart as two a year
    apart, and the patch communicates nothing to the operator writing a
    `min_capsem_version`.
    """
    version = _workspace_version()

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
    """The rule an operator reads must match the rule the release enforces."""
    skill = (PROJECT_ROOT / "skills" / "release-process" / "SKILL.md").read_text(encoding="utf-8")
    reference_name = "references/versions-and-commit-discipline.md"
    reference = (PROJECT_ROOT / "skills" / "release-process" / reference_name).read_text(
        encoding="utf-8"
    )

    assert reference_name in skill

    for required in (
        "Semver is mandatory",
        "min_capsem_version",
        "`<workspace version>-<first 12 hex",
        "`runtime-<channel>-<revision>`",
    ):
        assert required in reference, f"release version reference must document {required!r}"
