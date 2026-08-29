"""Own every macOS package, signing, and package-proof resource together."""

from __future__ import annotations

import stat
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
MACOS = ROOT / "build_system" / "packaging" / "macos"
LEGACY = ROOT / ("scr" + "ipts")

EXPECTED_MODES = {
    "build-pkg.sh": 0o755,
    "build-test-macos-package.sh": 0o755,
    "entitlements.plist": 0o644,
    "fix_p12_legacy.sh": 0o755,
    "install-local-macos-package.applescript": 0o644,
    "macos-install-user-request.sh": 0o755,
    "macos-tart-regression-probes.sh": 0o644,
    "macos_candidate_content.py": 0o644,
    "macos_release_glowup.py": 0o755,
    "macos_tart_glowup.py": 0o755,
    "macos_tart_guest.sh": 0o755,
    "macos_tart_transition_support.py": 0o644,
    "pkg-distribution.xml": 0o644,
    "pkg-scripts/install-user": 0o755,
    "pkg-scripts/postinstall": 0o755,
    "pkg-scripts/preinstall": 0o755,
    "prove-macos-package-boot.sh": 0o755,
    "run_signed.sh": 0o755,
}


def test_macos_packaging_resources_have_one_exact_owner() -> None:
    found = {
        path.relative_to(MACOS).as_posix()
        for path in MACOS.rglob("*")
        if path.is_file()
        and "__pycache__" not in path.parts
        and not path.name.startswith(".")
    }

    assert found == set(EXPECTED_MODES)
    assert not (ROOT / "entitlements.plist").exists()
    for name in EXPECTED_MODES:
        if name != "entitlements.plist":
            assert not (LEGACY / name).exists()


def test_macos_packaging_resources_preserve_reviewed_modes() -> None:
    for name, expected_mode in EXPECTED_MODES.items():
        path = MACOS / name
        assert path.is_file() and not path.is_symlink()
        assert stat.S_IMODE(path.stat().st_mode) == expected_mode


def test_gate_selects_macos_packaging_resources_from_their_owner() -> None:
    config = tomllib.loads((ROOT / "config" / "gate.toml").read_text(encoding="utf-8"))
    prefix = "build_system/packaging/macos/"

    assert config["install"]["local_macos_package_script"] == (
        prefix + "build-test-macos-package.sh"
    )
    assert config["install"]["local_macos_installer"] == [
        "/usr/bin/osascript",
        prefix + "install-local-macos-package.applescript",
    ]
    assert config["signing"]["entitlements"] == prefix + "entitlements.plist"
    assert config["modules"]["macos_glowup_script"] == (
        prefix + "macos_release_glowup.py"
    )


def test_macos_package_assembly_uses_owner_relative_resources() -> None:
    builder = (MACOS / "build-pkg.sh").read_text(encoding="utf-8")
    candidate = (MACOS / "build-test-macos-package.sh").read_text(encoding="utf-8")
    runner = (MACOS / "run_signed.sh").read_text(encoding="utf-8")

    assert 'bash "$SCRIPT_DIR/build-pkg.sh"' in candidate
    assert '"$SCRIPT_DIR/entitlements.plist"' in builder
    assert '"$SCRIPT_DIR/pkg-scripts/$package_script"' in builder
    assert '"$SCRIPT_DIR/../shared/$package_script"' in builder
    assert 'ENTITLEMENTS="$SCRIPT_DIR/entitlements.plist"' in runner


def test_release_workflow_selects_owned_macos_resources() -> None:
    source = (ROOT / ".github" / "workflows" / "release.yaml").read_text(
        encoding="utf-8"
    )
    prefix = "build_system/packaging/macos/"

    assert f"--entitlements {prefix}entitlements.plist" in source
    assert f"bash {prefix}build-pkg.sh" in source
    assert f"bash {prefix}macos-install-user-request.sh write" in source
