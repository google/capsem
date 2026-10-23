"""Verify guest environment configuration at runtime."""

import contextlib
import uuid

import psutil
import pytest
from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import exec_output_text, vm_session_dir, wait_exec_ready

pytestmark = pytest.mark.config_runtime


def test_env_var_injected(config_svc):
    """A private handoff injects the value without exposing it in owner argv."""
    client = config_svc.client()
    name = f"env-{uuid.uuid4().hex[:8]}"
    secret = f"guest-secret-{uuid.uuid4().hex}"

    def assert_guest_env_and_private_argv(vm_id):
        assert wait_exec_ready(client, vm_id, timeout=EXEC_READY_TIMEOUT)
        owners = [
            child for child in psutil.Process(config_svc.proc.pid).children(recursive=True)
            if child.name() == "capsem-process"
        ]
        assert owners, "expected a live VM owner process"
        for owner in owners:
            assert secret not in "\0".join(owner.cmdline())
        assert not (vm_session_dir(config_svc.tmp_dir, client, vm_id) / "guest-env.msgpack").exists()
        resp = client.post(f"/vms/{vm_id}/exec", {"command": "echo $TEST_VAR"})
        stdout = exec_output_text(resp) if resp else ""
        assert secret in stdout, f"Env var not found in guest: {stdout}"

    try:
        client.post("/vms/create", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
            "persistent": True, "env": {"TEST_VAR": secret},
        })
        assert_guest_env_and_private_argv(name)
        client.post(f"/vms/{name}/stop", {})
        resumed = client.post(f"/vms/{name}/resume", {})
        assert resumed is not None
        assert_guest_env_and_private_argv(resumed.get("id", name))

    finally:
        with contextlib.suppress(Exception):
            client.delete(f"/vms/{name}/delete")


def test_guest_has_python3(config_svc):
    """python3 is available in the guest."""
    client = config_svc.client()
    name = f"py3-{uuid.uuid4().hex[:8]}"

    try:
        client.post("/vms/create", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

        resp = client.post(f"/vms/{name}/exec", {"command": "python3 --version"})
        stdout = exec_output_text(resp) if resp else ""
        assert "Python 3" in stdout, f"python3 not available: {stdout}"

    finally:
        with contextlib.suppress(Exception):
            client.delete(f"/vms/{name}/delete")


def test_guest_arch_matches_host(config_svc):
    """Guest architecture matches the host (aarch64 on arm64 Mac)."""
    import os
    client = config_svc.client()
    name = f"arch-{uuid.uuid4().hex[:8]}"

    try:
        client.post("/vms/create", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

        resp = client.post(f"/vms/{name}/exec", {"command": "uname -m"})
        stdout = exec_output_text(resp).strip() if resp else ""

        host_arch = os.uname().machine
        if host_arch == "arm64":
            assert stdout == "aarch64", f"Expected aarch64, got {stdout}"
        else:
            assert stdout == "x86_64", f"Expected x86_64, got {stdout}"

    finally:
        with contextlib.suppress(Exception):
            client.delete(f"/vms/{name}/delete")
