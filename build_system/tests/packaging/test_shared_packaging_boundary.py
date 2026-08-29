"""Own the package-neutral resources shared by native packaging rails."""

from __future__ import annotations

import stat
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SHARED = ROOT / "build_system" / "packaging" / "shared"
LEGACY = ROOT / ("scr" + "ipts")

EXPECTED_RESOURCES = {
    "install-manifest-request.sh",
    "package_payload.py",
    "prepare-install-vm-devices.sh",
    "profile_root_payload.py",
}


def test_shared_packaging_resources_have_one_exact_owner() -> None:
    found = {
        path.name
        for path in SHARED.iterdir()
        if path.is_file() and not path.name.startswith(".")
    }

    assert found == EXPECTED_RESOURCES
    assert all(not (LEGACY / name).exists() for name in EXPECTED_RESOURCES)


def test_shared_packaging_resources_preserve_reviewed_source_modes() -> None:
    for name in EXPECTED_RESOURCES:
        path = SHARED / name
        assert path.is_file() and not path.is_symlink()
        assert stat.S_IMODE(path.stat().st_mode) == 0o644


def test_shared_shell_boundaries_preserve_usage_exit_status() -> None:
    for name, arguments in (
        ("install-manifest-request.sh", ("unsupported",)),
        ("prepare-install-vm-devices.sh", ()),
    ):
        result = subprocess.run(
            ("bash", str(SHARED / name), *arguments),
            capture_output=True,
            text=True,
            check=False,
        )
        assert result.returncode == 2, result.stderr


def test_gate_selects_shared_resources_from_their_owner() -> None:
    config = tomllib.loads((ROOT / "config" / "gate.toml").read_text(encoding="utf-8"))

    assert config["install"]["vm_device_setup_script"] == (
        "build_system/packaging/shared/prepare-install-vm-devices.sh"
    )
    assert config["install"]["request_script"] == (
        "build_system/packaging/shared/install-manifest-request.sh"
    )
    assert "build_system/packaging" in config["boundary"]["scripts"]["roots"]
    assert "build_system/packaging" in config["lint"]["python_roots"]


def test_release_workflow_uses_the_owned_manifest_handoff() -> None:
    source = (ROOT / ".github" / "workflows" / "release.yaml").read_text(
        encoding="utf-8"
    )
    owned = "build_system/packaging/shared/install-manifest-request.sh"
    legacy = "scr" + "ipts/install-manifest-request.sh"

    assert source.count(owned) == 5
    assert legacy not in source


def test_python_consumers_import_shared_helpers_from_their_owner() -> None:
    release_check = (LEGACY / "check-public-binary-release.py").read_text(
        encoding="utf-8"
    )
    profile_staging = (LEGACY / "stage-release-test-inputs.py").read_text(
        encoding="utf-8"
    )

    assert (
        "from build_system.packaging.shared.package_payload import package_payload_files"
        in release_check
    )
    assert (
        "from build_system.packaging.shared.profile_root_payload import stage_legacy_root"
        in profile_staging
    )
