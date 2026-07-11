"""Black-box contract tests for native installer failure diagnostics."""

from __future__ import annotations

import subprocess
from pathlib import Path


PROJECT_ROOT = Path(__file__).parent.parent
DIAGNOSTICS = PROJECT_ROOT / "scripts" / "pkg-scripts" / "install-diagnostics"


def test_install_failure_trap_writes_actionable_report_and_preserves_status(
    tmp_path: Path,
) -> None:
    run_log = tmp_path / "install-run.log"
    failure_report = tmp_path / "install-failure.txt"

    result = subprocess.run(
        [
            "/bin/bash",
            "-c",
            """
source "$1"
CAPSEM_INSTALL_PHASE="hydrate_assets"
CAPSEM_INSTALL_RUN_LOG="$2"
CAPSEM_INSTALL_FAILURE_FILE="$3"
CAPSEM_INSTALL_USER="tester"
CAPSEM_INSTALL_PRESENT_FAILURE=0
capsem_install_enable_failure_trap
exit 23
""",
            "install-diagnostics-test",
            str(DIAGNOSTICS),
            str(run_log),
            str(failure_report),
        ],
        capture_output=True,
        text=True,
        timeout=10,
    )

    assert result.returncode == 23
    report = failure_report.read_text()
    assert "Capsem installation failed." in report
    assert "Failed phase: hydrate_assets" in report
    assert "Exit code: 23" in report
    assert f"Detailed log: {run_log}" in report
    assert "Tester action: copy the output of this command into the bug report:" in report
    assert f'cat "{run_log}"' in report
    assert report in result.stderr


def test_successful_install_does_not_write_failure_report(tmp_path: Path) -> None:
    failure_report = tmp_path / "install-failure.txt"

    result = subprocess.run(
        [
            "/bin/bash",
            "-c",
            """
source "$1"
CAPSEM_INSTALL_PHASE="complete"
CAPSEM_INSTALL_RUN_LOG="$2"
CAPSEM_INSTALL_FAILURE_FILE="$3"
CAPSEM_INSTALL_USER="tester"
CAPSEM_INSTALL_PRESENT_FAILURE=0
capsem_install_enable_failure_trap
exit 0
""",
            "install-diagnostics-test",
            str(DIAGNOSTICS),
            str(tmp_path / "install-run.log"),
            str(failure_report),
        ],
        capture_output=True,
        text=True,
        timeout=10,
    )

    assert result.returncode == 0
    assert not failure_report.exists()
    assert result.stderr == ""


def test_macos_package_scripts_install_and_enable_failure_diagnostics() -> None:
    build_pkg = (PROJECT_ROOT / "scripts" / "build-pkg.sh").read_text()
    preinstall = (PROJECT_ROOT / "scripts" / "pkg-scripts" / "preinstall").read_text()
    postinstall = (PROJECT_ROOT / "scripts" / "pkg-scripts" / "postinstall").read_text()

    assert 'install -m 0755 "$SCRIPT_DIR/pkg-scripts/install-diagnostics"' in build_pkg
    for script in (preinstall, postinstall):
        assert 'source "$(dirname "$0")/install-diagnostics"' in script
        assert "capsem_install_enable_failure_trap" in script
        assert 'CAPSEM_INSTALL_FAILURE_FILE="$CAPSEM_DIR/logs/install-failure.txt"' in script
        assert 'CAPSEM_INSTALL_RUN_LOG="$INSTALL_RUN_LOG"' in script


def test_linux_package_scripts_embed_and_enable_failure_diagnostics() -> None:
    repack_deb = (PROJECT_ROOT / "scripts" / "repack-deb.sh").read_text()
    preinst = (PROJECT_ROOT / "scripts" / "deb-preinst.sh").read_text()
    postinst = (PROJECT_ROOT / "scripts" / "deb-postinst.sh").read_text()

    assert "embed_install_diagnostics" in repack_deb
    assert '"$SCRIPT_DIR/pkg-scripts/install-diagnostics"' in repack_deb
    for script in (preinst, postinst):
        assert "capsem_install_enable_failure_trap" in script
        assert 'CAPSEM_INSTALL_FAILURE_FILE="$CAPSEM_DIR/logs/install-failure.txt"' in script
        assert 'CAPSEM_INSTALL_RUN_LOG="$INSTALL_RUN_LOG"' in script
