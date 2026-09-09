"""Own every macOS package, signing, and package-proof resource together."""

from __future__ import annotations

import subprocess
import tomllib
from pathlib import Path

import pytest
from helpers.source_modes import tracked_source_modes

ROOT = Path(__file__).resolve().parents[3]
MACOS = ROOT / "build_system" / "packaging" / "macos"
LEGACY = ROOT / ("scr" + "ipts")


@pytest.mark.parametrize("status", [0, 23])
def test_physical_boot_cleanup_retains_logs_and_exit_status(tmp_path: Path, status: int) -> None:
    script = tmp_path / "build_system/packaging/macos/prove-macos-package-boot.sh"
    script.parent.mkdir(parents=True)
    # Run the real setup/EXIT trap without installing a package or starting VZ.
    setup = (MACOS / script.name).read_text().split('rm -rf "$WORK_ROOT"\n', 1)[0]
    script.write_text(setup + '''
mkdir -p "$RUN_DIR/persistent/vm" "$WORK_ROOT"
printf '%s' "$RUN_DIR" > "$WORK_ROOT/runtime-path"
printf 'checkpoint failed\\n' > "$RUN_DIR/persistent/vm/serial.log"
printf 'service evidence\\n' > "$RUN_DIR/service.2026-09-09.log"
printf 'large VM state is not diagnostic evidence' > "$RUN_DIR/persistent/vm/rootfs.img"
exit "$PROOF_EXIT_STATUS"
''')
    package = tmp_path / "fixture.pkg"
    package.write_text("fixture")
    result = subprocess.run(
        ["bash", "-c", 'uname() { echo Darwin; }; pkill() { :; }; source "$@"',
         "proof", str(script), "--package", str(package), "--version", "0.0.0",
         "--assets-dir", str(tmp_path)],
        env={"PATH": "/usr/bin:/bin", "PROOF_EXIT_STATUS": str(status)},
        capture_output=True, text=True, timeout=10,
    )
    assert result.returncode == status, result.stderr
    work = tmp_path / "cache/target/macos-package-boot"
    assert not Path((work / "runtime-path").read_text()).exists()
    logs = work / "diagnostics"
    assert (logs / "persistent/vm/serial.log").read_text() == "checkpoint failed\n"
    assert (logs / "service.2026-09-09.log").read_text() == "service evidence\n"
    assert not (logs / "persistent/vm/rootfs.img").exists()

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
    assert tracked_source_modes(ROOT, MACOS) == EXPECTED_MODES


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


def test_local_package_consumes_the_binaries_cargo_produced(tmp_path: Path) -> None:
    """Exercise the assembly handoff without compiling or installing a package."""
    script = tmp_path / "build_system/packaging/macos/build-test-macos-package.sh"
    script.parent.mkdir(parents=True)
    script.write_bytes((MACOS / script.name).read_bytes())
    (tmp_path / "Cargo.toml").write_text('version = "0.0.0"\n')
    release = tmp_path / "cache/target/cargo/release"
    (release / "bundle/macos/Capsem.app").mkdir(parents=True)
    (release / "capsem").write_text("compiled payload\n")
    content = tmp_path / "content"
    content.mkdir()

    result = subprocess.run(
        [
            "bash", "-c", r'''
            cargo() { :; }
            uname() { echo Darwin; }
            bash() {
                if [[ "${1##*/}" == build-pkg.sh ]]; then
                    [[ -d "$4" && -f "$5/capsem" ]] || {
                        echo "assembly cannot find Cargo's payload in $5" >&2
                        exit 1
                    }
                    echo PACKAGE_INPUTS_FOUND
                    exit 0
                fi
            }
            source "$1" --assets-dir "$2" --config-root "$2"
            ''',
            "package-handoff", str(script), str(content),
        ],
        capture_output=True, text=True, check=False, timeout=10,
    )

    assert result.returncode == 0, result.stdout + result.stderr
    assert "PACKAGE_INPUTS_FOUND" in result.stdout
