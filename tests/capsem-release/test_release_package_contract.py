from __future__ import annotations

import hashlib
import importlib.util
from pathlib import Path
from typing import Any

import blake3
import pytest
import yaml

PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = PROJECT_ROOT / "scripts" / "release-package-contract.py"
SPEC = importlib.util.spec_from_file_location("release_package_contract", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
CONTRACT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CONTRACT)


def _package(
    root: Path,
    *,
    name: str,
    platform: str,
    architecture: str,
    payload: bytes,
) -> dict[str, Any]:
    path = root / name
    path.write_bytes(payload)
    return {
        "name": name,
        "version": "0.6.2",
        "status": "current",
        "platform": platform,
        "architecture": architecture,
        "url": path.as_uri(),
        "digest": {
            "sha256": hashlib.sha256(payload).hexdigest(),
            "blake3": blake3.blake3(payload).hexdigest(),
        },
    }


def _manifest(root: Path) -> dict[str, Any]:
    return {
        "packages": [
            _package(
                root,
                name="Capsem-0.6.2.pkg",
                platform="macos",
                architecture="arm64",
                payload=b"macos package",
            ),
            _package(
                root,
                name="Capsem_0.6.2_amd64.deb",
                platform="linux",
                architecture="amd64",
                payload=b"amd64 package",
            ),
            _package(
                root,
                name="Capsem_0.6.2_arm64.deb",
                platform="linux",
                architecture="arm64",
                payload=b"arm64 package",
            ),
        ]
    }


def test_package_contract_verifies_the_exact_current_storage_cohort(tmp_path: Path) -> None:
    source = tmp_path / "source"
    source.mkdir()
    work = tmp_path / "verified"
    document = _manifest(source)

    assert (
        CONTRACT.verify_storage(
            document,
            expected_prefix=f"{source.as_uri()}/",
            expected_version="0.6.2",
            expected_count=3,
            work_dir=work,
        )
        == 3
    )
    assert sorted(path.name for path in work.iterdir()) == sorted(
        row["name"] for row in document["packages"]
    )


def test_package_contract_rejects_storage_outside_the_immutable_release(tmp_path: Path) -> None:
    source = tmp_path / "source"
    source.mkdir()
    with pytest.raises(ValueError, match="URL is outside"):
        CONTRACT.verify_storage(
            _manifest(source),
            expected_prefix="https://github.example.test/releases/download/v0.6.2/",
            expected_version="0.6.2",
            expected_count=3,
            work_dir=tmp_path / "verified",
        )


def test_selected_package_version_requires_one_current_platform_architecture(
    tmp_path: Path,
) -> None:
    source = tmp_path / "source"
    source.mkdir()
    document = _manifest(source)
    assert CONTRACT.selected_version(document, "linux", "amd64") == "0.6.2"
    document["packages"].append(dict(document["packages"][1]))
    with pytest.raises(ValueError, match="exactly one"):
        CONTRACT.selected_version(document, "linux", "amd64")


def test_release_and_recovery_share_publication_proof_scripts() -> None:
    workflows = [
        (PROJECT_ROOT / ".github/workflows/release.yaml").read_text(encoding="utf-8"),
        (PROJECT_ROOT / ".github/workflows/release-publication-recovery.yaml").read_text(
            encoding="utf-8"
        ),
    ]
    for workflow in workflows:
        assert "scripts/release-package-contract.py verify-storage" in workflow
        assert "scripts/prove-candidate-installer.sh" in workflow
        assert "scripts/prove-live-public-install.sh" in workflow
    live_proof = (PROJECT_ROOT / "scripts/prove-live-public-install.sh").read_text()
    assert "CAPSEM_LIVE_PUBLIC_INSTALL_SHELL_OK" in live_proof


def test_every_package_contract_caller_sets_up_uv_first() -> None:
    for workflow_name, job_name in (
        ("release.yaml", "verify-release-candidate"),
        ("release-publication-recovery.yaml", "recover-release-channel"),
    ):
        workflow = yaml.safe_load((PROJECT_ROOT / ".github/workflows" / workflow_name).read_text())
        steps = workflow["jobs"][job_name]["steps"]
        setup = next(
            index
            for index, step in enumerate(steps)
            if "astral-sh/setup-uv" in step.get("uses", "")
        )
        verify = next(
            index
            for index, step in enumerate(steps)
            if "scripts/release-package-contract.py verify-storage" in step.get("run", "")
        )
        assert setup < verify
