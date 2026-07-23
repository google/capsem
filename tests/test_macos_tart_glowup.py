"""Contracts for the clean-macOS Tart installed-package gate."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import subprocess
import sys

import pytest


PROJECT_ROOT = Path(__file__).resolve().parent.parent
HARNESS = PROJECT_ROOT / "scripts" / "macos_tart_glowup.py"
GLOWUP = PROJECT_ROOT / "scripts" / "macos_release_glowup.py"
GUEST = PROJECT_ROOT / "scripts" / "macos_tart_guest.sh"
HOST_BOOT = PROJECT_ROOT / "scripts" / "prove-macos-package-boot.sh"


def _load_harness():
    assert HARNESS.is_file(), "missing Tart macOS install harness"
    spec = importlib.util.spec_from_file_location("macos_tart_glowup", HARNESS)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def test_tart_commands_are_headless_isolated_and_share_only_gate_inputs(
    tmp_path: Path,
) -> None:
    module = _load_harness()
    share = tmp_path / "share"

    assert module.tart_run_command("capsem-glowup-123", share) == [
        "tart",
        "run",
        "--no-graphics",
        f"--dir=capsem-release:{share}",
        "capsem-glowup-123",
    ]
    assert module.tart_clone_command(
        "ghcr.io/cirruslabs/macos-sequoia-base:latest",
        "capsem-glowup-123",
    ) == [
        "tart",
        "clone",
        "ghcr.io/cirruslabs/macos-sequoia-base:latest",
        "capsem-glowup-123",
    ]
    assert module.tart_ip_command("capsem-glowup-123") == [
        "tart",
        "ip",
        "capsem-glowup-123",
        "--wait",
        "300",
    ]
    assert module.storage_control_command("tart-clean", "preflight") == [
        "uv",
        "run",
        "python",
        str(PROJECT_ROOT / "scripts" / "docker-storage-policy.py"),
        "tart-clean",
        "--label",
        "preflight",
    ]


def test_tart_harness_uses_the_declared_storage_policy() -> None:
    module = _load_harness()

    assert module.DEFAULT_IMAGE == "ghcr.io/cirruslabs/macos-sequoia-base:latest"
    assert module.OWNED_VM_PREFIX == "capsem-glowup-"


def test_tart_ssh_command_uses_quick_start_noninteractive_contract() -> None:
    module = _load_harness()

    command = module.ssh_command("192.168.64.7", ["uname", "-a"])

    assert command[:3] == ["sshpass", "-p", "admin"]
    assert "StrictHostKeyChecking=no" in command
    assert "UserKnownHostsFile=/dev/null" in command
    assert "ConnectTimeout=10" in command
    assert "IdentitiesOnly=yes" in command
    assert "PreferredAuthentications=password" in command
    assert "PubkeyAuthentication=no" in command
    assert "admin@192.168.64.7" in command
    assert command[-2:] == ["uname", "-a"]


def test_cleanup_refuses_to_stop_or_delete_foreign_tart_vms() -> None:
    module = _load_harness()
    calls: list[list[str]] = []

    def record(command: list[str], **_: object) -> subprocess.CompletedProcess[str]:
        calls.append(command)
        return subprocess.CompletedProcess(command, 0, "", "")

    with pytest.raises(ValueError, match="owned VM name"):
        module.cleanup_vm("developer-workstation", run=record)

    module.cleanup_vm("capsem-glowup-123", run=record)
    assert calls == [
        ["tart", "stop", "capsem-glowup-123"],
        ["tart", "delete", "capsem-glowup-123"],
    ]


def test_ip_wait_fails_immediately_when_tart_runner_exits() -> None:
    module = _load_harness()

    class ExitedRunner:
        def poll(self) -> int:
            return 64

    with pytest.raises(RuntimeError, match="runner exited before boot"):
        module.wait_for_guest_ip("capsem-glowup-123", ExitedRunner())


def test_tart_share_inputs_are_copied_not_hard_linked(tmp_path: Path) -> None:
    module = _load_harness()
    source = tmp_path / "source"
    destination = tmp_path / "destination"
    source.write_text("release input\n")
    source.chmod(0o755)

    module.stage_file(source, destination)

    assert destination.read_bytes() == source.read_bytes()
    assert destination.stat().st_ino != source.stat().st_ino
    assert destination.stat().st_mode & 0o777 == 0o755


def test_guest_installs_and_verifies_the_exact_shared_package() -> None:
    source = GUEST.read_text()

    assert '"/Volumes/My Shared Files/capsem-release/Capsem.pkg"' in source
    assert "/usr/sbin/installer -pkg" in source
    assert "pkgutil --pkg-info com.capsem.pkg" in source
    assert "/Applications/Capsem.app" in source
    assert 'CAPSEM_BIN_DIR="$CAPSEM_HOME/bin"' in source
    assert "verify-installed-release.py" in source
    assert "macos-install-user-request.sh" in source
    assert "capsem status" in source
    for binary in (
        "capsem",
        "capsem-service",
        "capsem-process",
        "capsem-tui",
        "capsem-mcp",
        "capsem-mcp-aggregator",
        "capsem-mcp-builtin",
        "capsem-gateway",
        "capsem-tray",
        "capsem-admin",
    ):
        assert binary in source


def test_physical_mac_boots_a_guest_from_the_exact_package_payload() -> None:
    source = HOST_BOOT.read_text()

    assert "pkgutil --expand-full" in source
    assert "scripts/simulate-install.sh" in source
    assert "scripts/prove-installed-shell.py" in source
    assert "CAPSEM_MACOS_PACKAGE_VM_BOOT_OK" in source
    assert '"guest_vm_booted": True' in source


def test_tart_harness_promotes_guest_evidence_to_a_durable_report() -> None:
    source = HARNESS.read_text()

    assert 'final_report_path = work_dir / "report.json"' in source
    assert "final_report_path.write_text(rendered_report)" in source
    assert 'run_storage_control("tart-clean", "macos-glowup-preflight")' in source
    assert '"macos-glowup-final"' in source


def test_bootstrap_doctor_and_canonical_gate_own_tart_without_polluting_smoke() -> None:
    bootstrap = (PROJECT_ROOT / "bootstrap.sh").read_text()
    doctor = (PROJECT_ROOT / "scripts" / "doctor-macos.sh").read_text()
    justfile = (PROJECT_ROOT / "justfile").read_text()

    assert "brew install cirruslabs/cli/tart cirruslabs/cli/sshpass" in bootstrap
    assert "brew trust --formula cirruslabs/cli/softnet" in bootstrap
    assert "tart --version" in doctor
    assert "sshpass" in doctor
    assert "test-macos-install:" not in justfile
    assert "python3 scripts/macos_release_glowup.py" in justfile

    test_start = justfile.index("test:")
    test_end = justfile.index("\n# Build the capsem-host-builder", test_start)
    canonical_gate = justfile[test_start:test_end]
    assert "python3 scripts/macos_release_glowup.py" in canonical_gate

    smoke_start = justfile.index("smoke:")
    smoke_end = justfile.index("\n# Gateway unit", smoke_start)
    smoke = justfile[smoke_start:smoke_end]
    assert "tart run" not in smoke.lower()
    assert "macos_tart_glowup.py" not in smoke
    assert "test-macos-install" not in smoke


def test_standalone_glowup_owns_build_tart_install_and_physical_boot() -> None:
    source = GLOWUP.read_text()

    assert '"scripts/build-test-macos-package.sh"' in source
    assert '"scripts/macos_tart_glowup.py"' in source
    assert '"scripts/prove-macos-package-boot.sh"' in source
    assert '"scripts/materialize-config.sh"' in source


def test_public_release_dispatch_recipe_is_gone() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()
    listed = subprocess.run(
        ["just", "--list", "--unsorted"],
        cwd=PROJECT_ROOT,
        text=True,
        capture_output=True,
        check=True,
    ).stdout

    assert '\nrelease tag="" channel="stable":' not in f"\n{justfile}"
    assert "    release " not in listed
    assert "qualify-release" in listed
    assert "cut-release" in listed
