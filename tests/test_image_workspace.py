from __future__ import annotations

from pathlib import Path

import pytest
from click.testing import CliRunner

from capsem.admin.cli import cli
from capsem.builder.config import load_guest_config
from capsem.builder.image_workspace import (
    ImageWorkspaceReport,
    dump_image_workspace_report_json,
    materialize_profile_image_workspace,
)
from capsem.builder.profiles import create_profile_draft, dump_profile_json


def _profile_with_packages():
    draft = create_profile_draft("corp-dev", revision="2026.0520.12")
    return draft.model_copy(
        update={
            "packages": draft.packages.model_copy(
                update={
                    "runtimes": {
                        "python": "3.12.3",
                        "node": "24.1.0",
                        "uv": "0.4.30",
                    },
                    "python_modules": {
                        "corp-sdk": "2.1.0",
                        "requests": "2.32.3",
                    },
                    "node_packages": {
                        "@corp/agent-kit": "7.8.9",
                    },
                    "system": draft.packages.system.model_copy(
                        update={
                            "apt": {
                                "ca-certificates": "20240203",
                                "curl": "8.11.1-1",
                            }
                        }
                    ),
                }
            ),
        }
    )


def test_materialize_profile_image_workspace_emits_valid_guest_config(tmp_path: Path) -> None:
    profile = _profile_with_packages()

    report = materialize_profile_image_workspace(profile, tmp_path, arch="arm64")
    dumped = dump_image_workspace_report_json(report)
    reparsed = ImageWorkspaceReport.model_validate_json(dumped)
    config = load_guest_config(tmp_path)

    assert report == reparsed
    assert report.schema_ == "capsem.image-workspace.v1"
    assert report.profile_id == "corp-dev"
    assert report.arches == ["arm64"]
    assert {file.path for file in report.files} >= {
        "profile.toml",
        "image-plan.json",
        "config/build.toml",
        "config/manifest.toml",
        "config/packages/apt.toml",
        "config/packages/python.toml",
        "config/packages/node.toml",
        "config/vm/resources.toml",
    }
    assert list(config.build.architectures) == ["arm64"]
    assert config.package_sets["apt"].packages == [
        "ca-certificates=20240203",
        "curl=8.11.1-1",
    ]
    assert config.package_sets["python"].packages == [
        "corp-sdk==2.1.0",
        "requests==2.32.3",
    ]
    assert config.package_sets["node"].packages == ["@corp/agent-kit@7.8.9"]
    assert config.vm_resources.cpu_count == profile.vm.cpus


def test_materialize_profile_image_workspace_uses_profile_not_repo_guest_config(
    tmp_path: Path,
) -> None:
    profile = _profile_with_packages()

    materialize_profile_image_workspace(profile, tmp_path)

    apt_toml = (tmp_path / "config" / "packages" / "apt.toml").read_text()
    python_toml = (tmp_path / "config" / "packages" / "python.toml").read_text()
    assert "corp-sdk==2.1.0" in python_toml
    assert "coreutils" not in apt_toml
    assert "pytest" not in python_toml


def test_materialize_profile_image_workspace_rejects_non_empty_output_without_force(
    tmp_path: Path,
) -> None:
    profile = _profile_with_packages()
    (tmp_path / "existing.txt").write_text("keep me\n", encoding="utf-8")

    with pytest.raises(FileExistsError, match="pass --force"):
        materialize_profile_image_workspace(profile, tmp_path)

    report = materialize_profile_image_workspace(profile, tmp_path, force=True)

    assert report.profile_id == "corp-dev"
    assert (tmp_path / "existing.txt").exists()


def test_capsem_admin_image_build_workspace_outputs_typed_report(tmp_path: Path) -> None:
    profile = _profile_with_packages()
    profile_path = tmp_path / "profile.json"
    profile_path.write_text(dump_profile_json(profile), encoding="utf-8")
    workspace = tmp_path / "workspace"

    result = CliRunner().invoke(
        cli,
        [
            "image",
            "build-workspace",
            str(profile_path),
            "--out",
            str(workspace),
            "--arch",
            "x86_64",
            "--json",
        ],
    )

    assert result.exit_code == 0, result.output
    assert '"schema": "capsem.image-workspace.v1"' in result.output
    assert '"arches": [' in result.output
    assert '"x86_64"' in result.output
    assert (workspace / "config" / "build.toml").exists()
    assert list(load_guest_config(workspace).build.architectures) == ["x86_64"]
