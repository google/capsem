"""A real supervisor restarts the gateway only after the SDK receives its reply."""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

import pytest
from helpers.constants import ASSETS_DIR
from helpers.managed_service import managed_service
from helpers.service import ServiceInstance
from helpers.stopped_workspace import VM_ID, seed_stopped_workspace

ROOT = Path(__file__).resolve().parents[2]


@contextmanager
def unrelated_manager_defaults() -> Iterator[None]:
    """The service must use its authenticated owner, not manager defaults."""
    if sys.platform != "linux":
        yield
        return
    from helpers.systemd_manager import manager_pid

    manager_pid(Path(os.environ["XDG_RUNTIME_DIR"]), same_network=True)
    command = ["systemctl", "--user"]
    keys = ("XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS")
    previous = subprocess.run(
        [*command, "show-environment"], check=True, capture_output=True, text=True, timeout=5,
    )
    restore = [line for line in previous.stdout.splitlines() if line.partition("=")[0] in keys]
    try:
        subprocess.run([
            *command, "set-environment", "XDG_RUNTIME_DIR=/nonexistent/capsem-runtime",
            "DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent/capsem-bus",
        ], check=True, capture_output=True, timeout=5)
        yield
    finally:
        subprocess.run([*command, "unset-environment", *keys], check=True, capture_output=True, timeout=5)
        if restore:
            subprocess.run([*command, "set-environment", *restore], check=True, capture_output=True, timeout=5)


def test_unbound_systemd_supervisor_cannot_touch_an_ambient_unit(monkeypatch: pytest.MonkeyPatch) -> None:
    from helpers.managed_service import SystemdService

    def unexpected_command(*_args: object, **_kwargs: object) -> None:
        raise AssertionError("unbound supervisor attempted a systemd command")

    owner = SystemdService(ServiceInstance(sign_binaries=False))
    monkeypatch.setattr(subprocess, "run", unexpected_command)
    owner.retire()
    assert not owner.active()
    with pytest.raises(AssertionError, match="supervisor manager has not been bound"):
        owner.command("show", owner.label)


@pytest.mark.integration
@pytest.mark.skipif(sys.platform != "linux", reason="Linux user-manager handoff")
def test_supervisor_worker_keeps_explicit_manager_across_environment_changes(tmp_path: Path) -> None:
    from helpers.systemd_manager import manager_pid

    runtime = os.environ["XDG_RUNTIME_DIR"]
    manager = manager_pid(Path(runtime))
    result = subprocess.run([
        sys.executable, "-m", "tests.helpers.run_native_restart", "--inside",
        "--uid", str(os.getuid()), "--gid", str(os.getgid()),
        "--manager-runtime", runtime, "--manager-pid", str(manager), "--",
        sys.executable, "-c",
        "import os,sys; "
        "assert os.environ['XDG_RUNTIME_DIR'] == sys.argv[1]; "
        "assert os.environ['DBUS_SESSION_BUS_ADDRESS'] == 'unix:path=' + sys.argv[1] + '/bus'; "
        "print('SDK_SUPERVISOR_MANAGER_HANDOFF_OK')", runtime,
    ], cwd=ROOT, env={
        **os.environ,
        "XDG_RUNTIME_DIR": str(tmp_path / "unrelated-runtime"),
        "DBUS_SESSION_BUS_ADDRESS": f"unix:path={tmp_path / 'unrelated-bus'}",
    }, capture_output=True, text=True, timeout=10, check=False)
    assert result.returncode == 0, result.stdout + result.stderr
    assert "SDK_SUPERVISOR_KERNEL_PROOF_OK" in result.stdout
    assert "SDK_SUPERVISOR_MANAGER_HANDOFF_OK" in result.stdout


@pytest.mark.integration
@pytest.mark.skipif(sys.platform != "linux", reason="Linux user-manager handoff")
def test_supervisor_worker_refuses_a_different_manager_pid() -> None:
    result = subprocess.run([
        sys.executable, "-m", "tests.helpers.run_native_restart", "--inside",
        "--uid", str(os.getuid()), "--gid", str(os.getgid()),
        "--manager-runtime", os.environ["XDG_RUNTIME_DIR"], "--manager-pid", str(os.getpid()),
        "--", sys.executable, "-c", "print('UNEXPECTED_KERNEL_COMMAND')",
    ], cwd=ROOT, capture_output=True, text=True, timeout=10, check=False)
    assert result.returncode != 0
    assert "AssertionError: native supervisor manager PID changed" in result.stderr
    assert "SDK_SUPERVISOR_KERNEL_PROOF_OK" not in result.stdout
    assert "UNEXPECTED_KERNEL_COMMAND" not in result.stdout


def workspace_digest(root: Path) -> dict[str, str]:
    return {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in (root / "guest/workspace").rglob("*") if path.is_file()}


@pytest.mark.integration
@pytest.mark.parametrize("language", ["python", "typescript"])
def test_sdk_receives_managed_restart_and_reconnects_explicitly(language: str) -> None:
    service = ServiceInstance()
    seed_stopped_workspace(service.tmp_dir, ASSETS_DIR)
    with unrelated_manager_defaults(), managed_service(service) as managed:
        before = managed.ready()
        assert (service.tmp_dir / "managed-service.log").is_file(), (
            "the native supervisor must retain startup output for failed hosted acceptance"
        )
        if sys.platform == "linux":
            assert os.readlink(f"/proc/{before.service_pid}/ns/net") == os.readlink("/proc/self/ns/net")
            assert "NoNewPrivs:\t1" in Path(f"/proc/{before.service_pid}/status").read_text()
        status, inventory = before.get("/vms/list")
        assert status == 200
        registry = json.loads((service.tmp_dir / "persistent_registry.json").read_text())
        workspace = workspace_digest(service.tmp_dir / "persistent" / VM_ID)
        project = ROOT / "sdk" / language
        command = ["uv", "run", "--frozen", "--no-sync", "python", "-m", "tests.restart_acceptance"] if language == "python" else ["node", "tools/restart-acceptance.mjs"]
        result = subprocess.run(command, cwd=project, env={
            **{key: value for key, value in os.environ.items() if key != "VIRTUAL_ENV"},
            "SDK_GATEWAY_URL": f"http://127.0.0.1:{before.port}", "SDK_GATEWAY_TOKEN": before.token,
            "CAPSEM_RUN_DIR": str(service.tmp_dir),
        }, capture_output=True, text=True, timeout=20, check=False)
        assert result.returncode == 0, result.stdout + result.stderr
        assert "SDK_RESTART_ACCEPTED" in result.stdout
        after = managed.ready(before)
        if sys.platform == "linux":
            assert os.readlink(f"/proc/{after.service_pid}/ns/net") == os.readlink("/proc/self/ns/net")
            assert "NoNewPrivs:\t1" in Path(f"/proc/{after.service_pid}/status").read_text()
        assert after.get("/vms/list", token=before.token)[0] == 401
        assert after.get("/vms/list") == (200, inventory)
        assert json.loads((service.tmp_dir / "persistent_registry.json").read_text()) == registry
        assert workspace_digest(service.tmp_dir / "persistent" / VM_ID) == workspace
        reconnect = subprocess.run(
            ["uv", "run", "--frozen", "--no-sync", "python", "-m", "tests.gateway_acceptance"]
            if language == "python" else ["node", "tools/gateway-acceptance.mjs"],
            cwd=project, env={
                **{key: value for key, value in os.environ.items() if key != "VIRTUAL_ENV"},
                "SDK_GATEWAY_URL": f"http://127.0.0.1:{after.port}", "SDK_GATEWAY_TOKEN": after.token,
                "SDK_VM_ID": VM_ID, "CAPSEM_RUN_DIR": str(service.tmp_dir),
            }, capture_output=True, text=True, timeout=30, check=False,
        )
        assert reconnect.returncode == 0, reconnect.stdout + reconnect.stderr
        assert "BRAAVOS_SDK_ACCEPTANCE_OK" in reconnect.stdout
        assert "SDK_LOCAL_DISCOVERY_OK" in reconnect.stdout
        print(f"{language}: acknowledgement delivered; service and gateway PIDs changed; token rotated; stopped VM unchanged")


@pytest.mark.integration
def test_supervisor_worker_refuses_a_forged_native_uid() -> None:
    result = subprocess.run([
        sys.executable, "-m", "tests.helpers.run_native_restart", "--inside",
        "--uid", str(os.getuid() + 1), "--gid", str(os.getgid()), "--",
        sys.executable, "-c", "print('UNEXPECTED_KERNEL_COMMAND')",
    ], cwd=ROOT, capture_output=True, text=True, timeout=10, check=False)
    assert result.returncode != 0
    assert "AssertionError: native supervisor UID/GID changed" in result.stderr
    assert "SDK_SUPERVISOR_KERNEL_PROOF_OK" not in result.stdout
    assert "UNEXPECTED_KERNEL_COMMAND" not in result.stdout
