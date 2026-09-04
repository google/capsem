"""Own the package-neutral resources shared by native packaging rails."""

from __future__ import annotations

import subprocess
import tomllib
from pathlib import Path

from helpers.source_modes import tracked_source_modes

ROOT = Path(__file__).resolve().parents[3]
SHARED = ROOT / "build_system" / "packaging" / "shared"
LEGACY = ROOT / ("scr" + "ipts")
LINUX = ROOT / "build_system" / "packaging" / "linux"

EXPECTED_RESOURCE_MODES = {
    "install-diagnostics": 0o755,
    "install-vm-device-access": 0o644,
    "install-manifest": 0o644,
    "install-manifest-request.sh": 0o644,
    "package_payload.py": 0o644,
    "prepare-install-vm-devices.sh": 0o644,
    "profile_root_payload.py": 0o644,
    "retire-cohort": 0o755,
    "service-owned-update": 0o644,
}

LEGACY_LIFECYCLE = LEGACY / "pkg-scripts"
SHARED_LIFECYCLE = {
    "install-diagnostics",
    "install-manifest",
    "retire-cohort",
    "service-owned-update",
}


def test_shared_packaging_resources_have_one_exact_owner() -> None:
    found = {
        path.name
        for path in SHARED.iterdir()
        if path.is_file() and not path.name.startswith(".")
    }

    assert found == set(EXPECTED_RESOURCE_MODES)
    assert all(
        not (LEGACY / name).exists()
        for name in EXPECTED_RESOURCE_MODES
        if name.endswith((".py", ".sh"))
    )
    assert all(not (LEGACY_LIFECYCLE / name).exists() for name in SHARED_LIFECYCLE)


def test_shared_packaging_resources_preserve_reviewed_source_modes() -> None:
    assert tracked_source_modes(ROOT, SHARED) == EXPECTED_RESOURCE_MODES


def test_native_package_rails_select_shared_lifecycle_helpers() -> None:
    macos_builder = (
        ROOT / "build_system/packaging/macos/build-pkg.sh"
    ).read_text(encoding="utf-8")
    linux_repacker = (LINUX / "repack-deb.sh").read_text(encoding="utf-8")
    linux_preinstall = (LINUX / "deb-preinst.sh").read_text(encoding="utf-8")
    linux_postinstall = (LINUX / "deb-postinst.sh").read_text(encoding="utf-8")

    assert '$SCRIPT_DIR/../shared/$package_script' in macos_builder
    assert '$SCRIPT_DIR/../shared/$helper' in linux_repacker
    for maintainer_script in (linux_preinstall, linux_postinstall):
        assert '$(dirname "$0")/../shared/' in maintainer_script


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


def test_embedded_vm_device_helper_does_not_consume_maintainer_arguments(
    tmp_path: Path,
) -> None:
    helper = (SHARED / "install-vm-device-access").read_text(encoding="utf-8")
    postinstall = tmp_path / "postinst"
    postinstall.write_text(
        "#!/bin/bash\n" + helper.split("\n", 1)[1] + '\nprintf "postinst:%s\\n" "$1"\n',
        encoding="utf-8",
    )

    result = subprocess.run(
        ("bash", str(postinstall), "configure"), capture_output=True, text=True, check=False
    )

    assert result.returncode == 0, result.stderr
    assert result.stdout == "postinst:configure\n"


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
    release_check = (
        ROOT / "build_system/builder/release/tools/check_public_binary_release.py"
    ).read_text(encoding="utf-8")
    profile_staging = (
        ROOT / "build_system/builder/release/tools/stage_release_test_inputs.py"
    ).read_text(encoding="utf-8")

    assert (
        "from .package_payload import package_payload_files"
        in release_check
    )
    assert (
        "from .profile_root_payload import stage_legacy_root"
        in profile_staging
    )
    compatibility = (SHARED / "profile_root_payload.py").read_text(encoding="utf-8")
    assert "from capsem_builder.release.tools.profile_root_payload import *" in compatibility
