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
from typing import Any

import pytest

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


def _workspace() -> dict[str, Any]:
    return tomllib.loads((PROJECT_ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]


def _workspace_version() -> str:
    return _workspace()["package"]["version"]


def _sdk_publication_manifests() -> set[Path]:
    """Cargo needs registry versions for the SDK's local library dependencies."""
    pending = [(PROJECT_ROOT / "sdk/rust/Cargo.toml").resolve()]
    published = set()
    while pending:
        cargo_toml = pending.pop()
        if cargo_toml in published:
            continue
        assert cargo_toml.is_relative_to(PROJECT_ROOT), cargo_toml
        published.add(cargo_toml)
        manifest = tomllib.loads(cargo_toml.read_text(encoding="utf-8"))
        for section in ("dependencies", "build-dependencies"):
            for name, spec in (manifest.get(section) or {}).items():
                if (
                    isinstance(spec, dict)
                    and str(spec.get("package", name)).startswith("capsem")
                    and "path" in spec
                ):
                    pending.append((cargo_toml.parent / str(spec["path"]) / "Cargo.toml").resolve())
    return published


def _dependency_version_errors(
    manifest: dict[str, Any], *, published: bool, version: str
) -> list[str]:
    errors = []
    for section in ("dependencies", "dev-dependencies", "build-dependencies"):
        for alias, spec in (manifest.get(section) or {}).items():
            name = str(spec.get("package", alias)) if isinstance(spec, dict) else alias
            if not name.startswith("capsem"):
                continue
            if published and section != "dev-dependencies":
                if (
                    not isinstance(spec, dict)
                    or not spec.get("path")
                    or spec.get("version") != version
                ):
                    errors.append(
                        f"{section}: {alias} must use a path and workspace version {version}"
                    )
            elif isinstance(spec, str) or (isinstance(spec, dict) and "version" in spec):
                errors.append(f"{section}: private dependency {alias} must not pin a version")
    return errors


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
    """Application deps stay path-only; published SDK deps match the cohort.

    A pinned internal version is a second place the workspace version lives,
    and it drifts silently: `capsem-guard = { version = "1.0.1776688771" }`
    sat unnoticed for months because caret matching accepted every 1.x, then
    broke the entire workspace build the moment the line moved to 0.6.
    """
    published = _sdk_publication_manifests()
    offenders = []
    for member in _workspace()["members"]:
        cargo_toml = (PROJECT_ROOT / member / "Cargo.toml").resolve()
        manifest = tomllib.loads(cargo_toml.read_text(encoding="utf-8"))
        offenders.extend(
            f"{member}/{error}"
            for error in _dependency_version_errors(
                manifest, published=cargo_toml in published, version=_workspace_version()
            )
        )

    assert not offenders, (
        "Application dependencies must be path-only; the published SDK library graph "
        "must declare registry constraints matching the authoritative workspace version:\n  "
        + "\n  ".join(offenders)
    )


@pytest.mark.parametrize("defect", ["missing", "drift", "private", "alias"])
def test_dependency_version_guard_rejects_publication_and_drift_defects(defect):
    crate = "capsem-core" if defect == "private" else "capsem-api"
    cargo_toml = (PROJECT_ROOT / "crates" / crate / "Cargo.toml").resolve()
    manifest = tomllib.loads(cargo_toml.read_text(encoding="utf-8"))
    dependencies = manifest["dependencies"]
    name = next(name for name in dependencies if name.startswith("capsem"))
    spec = dependencies[name]
    if defect == "missing":
        spec.pop("version")
    elif defect == "private":
        spec["version"] = _workspace_version()
    else:
        spec["version"] = "0.0.0"
        if defect == "alias":
            spec["package"] = name
            dependencies["renamed_internal"] = dependencies.pop(name)
    assert _dependency_version_errors(
        manifest,
        published=cargo_toml in _sdk_publication_manifests(),
        version=_workspace_version(),
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
