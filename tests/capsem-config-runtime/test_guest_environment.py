"""Verify guest environment configuration at runtime."""

import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import wait_exec_ready

pytestmark = pytest.mark.config_runtime


def test_env_var_injected(config_svc):
    """Environment variable passed via --env appears inside the guest."""
    client = config_svc.client()
    name = f"env-{uuid.uuid4().hex[:8]}"

    try:
        client.post("/provision", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
            "env": {"TEST_VAR": "hello_from_host"},
        })
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

        resp = client.post(f"/exec/{name}", {"command": "echo $TEST_VAR"})
        stdout = resp.get("stdout", "") if resp else ""
        assert "hello_from_host" in stdout, f"Env var not found in guest: {stdout}"

    finally:
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass


def test_guest_has_python3(config_svc):
    """python3 is available in the guest."""
    client = config_svc.client()
    name = f"py3-{uuid.uuid4().hex[:8]}"

    try:
        client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

        resp = client.post(f"/exec/{name}", {"command": "python3 --version"})
        stdout = resp.get("stdout", "") if resp else ""
        assert "Python 3" in stdout, f"python3 not available: {stdout}"

    finally:
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass


def test_guest_arch_matches_host(config_svc):
    """Guest architecture matches the host (aarch64 on arm64 Mac)."""
    import os
    client = config_svc.client()
    name = f"arch-{uuid.uuid4().hex[:8]}"

    try:
        client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

        resp = client.post(f"/exec/{name}", {"command": "uname -m"})
        stdout = resp.get("stdout", "").strip() if resp else ""

        host_arch = os.uname().machine
        if host_arch == "arm64":
            assert stdout == "aarch64", f"Expected aarch64, got {stdout}"
        else:
            assert stdout == "x86_64", f"Expected x86_64, got {stdout}"

    finally:
        try:
            client.delete(f"/delete/{name}")
        except Exception:
            pass
