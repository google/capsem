from __future__ import annotations

from pathlib import Path

from click.testing import CliRunner

from capsem.admin.cli import cli
from capsem.builder.image_plan import derive_image_plan
from capsem.builder.image_sbom import (
    SpdxDocument,
    dump_spdx_document_json,
    generate_image_spdx_document,
)
from capsem.builder.image_verify import ImageInventory, dump_image_inventory_json
from capsem.builder.profiles import create_profile_draft, dump_profile_json


def _profile_path(tmp_path: Path) -> Path:
    profile = create_profile_draft("corp-dev", revision="2026.0521.1")
    path = tmp_path / "corp-dev.profile.json"
    path.write_text(dump_profile_json(profile), encoding="utf-8")
    return path


def _inventory() -> ImageInventory:
    return ImageInventory(
        apt={"coreutils": "9.1-1", "curl": "8.0.0"},
        python_modules={"pytest": "8.3.5"},
        node_packages={"@openai/codex": "0.1.0"},
        tools={"capsem_doctor": "2026.05.20"},
    )


def _write_inventory(path: Path, inventory: ImageInventory | None = None) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        dump_image_inventory_json(inventory or _inventory()),
        encoding="utf-8",
    )


def test_generate_image_spdx_document_from_inventory() -> None:
    profile = create_profile_draft("corp-dev", revision="2026.0521.1")
    plan = derive_image_plan(profile, arch="arm64")

    document = generate_image_spdx_document(
        plan,
        "arm64",
        _inventory(),
        created="2026-05-21T12:00:00Z",
    )
    dumped = dump_spdx_document_json(document)
    reparsed = SpdxDocument.model_validate_json(dumped)

    assert reparsed == document
    assert document.spdx_version == "SPDX-2.3"
    assert "corp-dev" in document.document_namespace
    assert "2026.0521.1" in document.document_namespace
    assert {package.name for package in document.packages} == {
        "@openai/codex",
        "capsem_doctor",
        "coreutils",
        "curl",
        "pytest",
    }
    purls = [
        ref.reference_locator
        for package in document.packages
        for ref in package.external_refs
    ]
    assert "pkg:pypi/pytest@8.3.5" in purls
    assert "pkg:npm/%40openai/codex@0.1.0" in purls
    assert any(purl.startswith("pkg:deb/debian/coreutils@9.1-1") for purl in purls)


def test_capsem_admin_image_sbom_writes_single_arch_spdx_stdout(tmp_path: Path) -> None:
    profile_path = _profile_path(tmp_path)
    assets_dir = tmp_path / "assets"
    _write_inventory(assets_dir / "arm64" / "image-inventory.json")

    result = CliRunner().invoke(
        cli,
        [
            "image",
            "sbom",
            str(profile_path),
            "--assets-dir",
            str(assets_dir),
            "--arch",
            "arm64",
        ],
    )

    assert result.exit_code == 0, result.output
    document = SpdxDocument.model_validate_json(result.output)
    assert document.spdx_version == "SPDX-2.3"
    assert "arm64 guest image SBOM" in document.name


def test_capsem_admin_image_sbom_writes_all_arch_outputs(tmp_path: Path) -> None:
    profile_path = _profile_path(tmp_path)
    assets_dir = tmp_path / "assets"
    _write_inventory(assets_dir / "arm64" / "image-inventory.json")
    _write_inventory(assets_dir / "x86_64" / "image-inventory.json")
    out_dir = tmp_path / "sboms"

    result = CliRunner().invoke(
        cli,
        [
            "image",
            "sbom",
            str(profile_path),
            "--assets-dir",
            str(assets_dir),
            "--out-dir",
            str(out_dir),
        ],
    )

    assert result.exit_code == 0, result.output
    arm = SpdxDocument.model_validate_json(
        (out_dir / "arm64" / "guest-sbom.spdx.json").read_text(encoding="utf-8")
    )
    x86 = SpdxDocument.model_validate_json(
        (out_dir / "x86_64" / "guest-sbom.spdx.json").read_text(encoding="utf-8")
    )
    assert "arm64" in arm.name
    assert "x86_64" in x86.name


def test_capsem_admin_image_sbom_rejects_all_arch_stdout(tmp_path: Path) -> None:
    profile_path = _profile_path(tmp_path)
    assets_dir = tmp_path / "assets"
    _write_inventory(assets_dir / "arm64" / "image-inventory.json")
    _write_inventory(assets_dir / "x86_64" / "image-inventory.json")

    result = CliRunner().invoke(
        cli,
        ["image", "sbom", str(profile_path), "--assets-dir", str(assets_dir)],
    )

    assert result.exit_code == 1
    assert "all-arch SBOM output requires --out-dir" in result.output


def test_capsem_admin_image_sbom_requires_selected_arch_inventory(tmp_path: Path) -> None:
    profile_path = _profile_path(tmp_path)
    assets_dir = tmp_path / "assets"
    assets_dir.mkdir()

    result = CliRunner().invoke(
        cli,
        [
            "image",
            "sbom",
            str(profile_path),
            "--assets-dir",
            str(assets_dir),
            "--arch",
            "arm64",
        ],
    )

    assert result.exit_code == 1
    assert "missing image inventory for arch(es): arm64" in result.output
