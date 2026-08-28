"""The release summary, which is a program and was written inside YAML.

`create-release` built its step summary with a shell body: a `find`, two `du`
calls, an embedded Python one-liner for the SBOM count, a loop accumulating
markdown rows, and `[ -n "$LINUX_ROWS" ]` -- an assertion that fails the release
when no Linux package reached the job. Twenty-five executable lines no test
could call, running after attestation and before the GitHub release is created.

The assertion is the part worth keeping and the part worth testing: a release
whose artifacts are missing a `.deb` must stop, and say which.
"""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[3]
SCRIPT = PROJECT_ROOT / "scripts" / "write-release-summary.py"


def _module():
    spec = importlib.util.spec_from_file_location("write_release_summary", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _artifacts(root: Path, *, debs: tuple[str, ...], pkg: str | None, sbom: int | None) -> Path:
    root.mkdir(parents=True, exist_ok=True)
    for name in debs:
        (root / name).write_bytes(b"x" * 2048)
    if pkg:
        (root / pkg).write_bytes(b"y" * 4096)
    if sbom is not None:
        (root / "capsem-sbom.spdx.json").write_text(
            json.dumps({"packages": [{"name": f"p{i}"} for i in range(sbom)]}), encoding="utf-8"
        )
    return root


def test_a_summary_names_every_artifact_that_shipped(tmp_path: Path) -> None:
    module = _module()
    root = _artifacts(
        tmp_path / "release-artifacts",
        debs=("Capsem_9.9.9_amd64.deb", "Capsem_9.9.9_arm64.deb"),
        pkg="Capsem-9.9.9.pkg",
        sbom=42,
    )

    summary = module.render(root, tag="v9.9.9", manifest_url="https://example/manifest.json")

    assert "## Release 9.9.9" in summary
    assert "Capsem-9.9.9.pkg" in summary
    assert "Capsem_9.9.9_amd64.deb" in summary
    assert "Capsem_9.9.9_arm64.deb" in summary
    assert "42 packages" in summary
    assert "https://example/manifest.json" in summary


def test_a_release_with_no_linux_package_is_refused(tmp_path: Path) -> None:
    """The assertion the shell body carried, kept and now reachable.

    A release that reaches this step without a `.deb` has lost an artifact
    between jobs, and publishing the rest would ship a channel whose Linux rows
    point at nothing.
    """
    module = _module()
    root = _artifacts(tmp_path / "release-artifacts", debs=(), pkg="Capsem-9.9.9.pkg", sbom=1)

    with pytest.raises(SystemExit, match="no Linux package"):
        module.render(root, tag="v9.9.9", manifest_url="https://example/manifest.json")


def test_a_missing_sbom_is_reported_rather_than_guessed(tmp_path: Path) -> None:
    """The shell wrote `?` on any error, which reads as a rendering quirk.

    The SBOM is attested one step earlier, so its absence here means the
    attestation covered something this summary cannot see.
    """
    module = _module()
    root = _artifacts(
        tmp_path / "release-artifacts", debs=("Capsem_9.9.9_amd64.deb",), pkg=None, sbom=None
    )

    with pytest.raises(SystemExit, match="SBOM"):
        module.render(root, tag="v9.9.9", manifest_url="https://example/manifest.json")
