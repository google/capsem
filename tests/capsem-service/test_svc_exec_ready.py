"""Server-side VM readiness tests.

Regression tests for the race condition where provision/resume returns
before the VM socket is ready, causing immediate exec/write/read calls
to fail with "failed to connect to sandbox".

These tests deliberately do NOT use wait_exec_ready() or the ready_vm
fixture. The server must handle readiness internally -- clients should
not need to poll.
"""

import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_TIMEOUT_SECS, HTTP_TIMEOUT

pytestmark = pytest.mark.integration


def vm_name(prefix="r"):
    return f"{prefix}-{uuid.uuid4().hex[:6]}"


class TestExecImmediatelyAfterProvision:
    """Provision a VM, then immediately call endpoints without polling."""

    def test_exec_immediately_after_provision(self, service_env):
        """POST /exec/{id} must succeed right after POST /provision."""
        client = service_env.client()
        name = vm_name("ei")
        resp = client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        assert resp is not None, "provision failed"
        vm_id = resp.get("id", name)

        # Immediately exec -- no wait_exec_ready, no sleep.
        # The server must internally wait for the VM to be ready.
        exec_resp = client.post(
            f"/exec/{vm_id}",
            {"command": "echo ready-no-wait", "timeout_secs": EXEC_TIMEOUT_SECS},
            timeout=HTTP_TIMEOUT,
        )
        assert exec_resp is not None, "exec returned None"
        assert "ready-no-wait" in exec_resp.get("stdout", ""), (
            f"expected 'ready-no-wait' in stdout, got: {exec_resp}"
        )
        assert exec_resp.get("exit_code") == 0

        client.delete(f"/delete/{vm_id}")

    def test_write_file_immediately_after_provision(self, service_env):
        """POST /write_file/{id} must succeed right after POST /provision."""
        client = service_env.client()
        name = vm_name("wi")
        resp = client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        assert resp is not None
        vm_id = resp.get("id", name)

        # Immediately write -- server must wait for VM readiness.
        write_resp = client.post(
            f"/write_file/{vm_id}",
            {"path": "/root/race-test.txt", "content": "race-check"},
            timeout=HTTP_TIMEOUT,
        )
        assert write_resp is not None, "write_file returned None"
        assert write_resp.get("success") is True, f"write_file failed: {write_resp}"

        client.delete(f"/delete/{vm_id}")

    def test_read_file_immediately_after_provision(self, service_env):
        """POST /write_file + /read_file must succeed right after POST /provision."""
        client = service_env.client()
        name = vm_name("ri")
        resp = client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        assert resp is not None
        vm_id = resp.get("id", name)

        # Immediately write then read -- server must wait for VM readiness.
        write_resp = client.post(
            f"/write_file/{vm_id}",
            {"path": "/root/read-probe.txt", "content": "probe-data"},
            timeout=HTTP_TIMEOUT,
        )
        assert write_resp is not None, "write_file returned None"

        read_resp = client.post(
            f"/read_file/{vm_id}",
            {"path": "/root/read-probe.txt"},
            timeout=HTTP_TIMEOUT,
        )
        assert read_resp is not None, "read_file returned None"
        assert "content" in read_resp, f"read_file missing content: {read_resp}"

        client.delete(f"/delete/{vm_id}")


class TestExecImmediatelyAfterResume:
    """Stop a persistent VM, resume it, then immediately exec."""

    def test_exec_immediately_after_resume(self, service_env):
        """POST /exec/{name} must succeed right after POST /resume/{name}."""
        client = service_env.client()
        name = vm_name("rs")

        # 1. Provision a persistent VM. Server-side wait means this
        #    exec will block until VM is ready (no client poll needed).
        prov_resp = client.post("/provision", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True,
        })
        assert prov_resp is not None and "error" not in prov_resp, (
            f"provision persistent VM failed: {prov_resp}"
        )
        setup_resp = client.post(
            f"/exec/{name}",
            {"command": "echo setup-ok", "timeout_secs": EXEC_TIMEOUT_SECS},
            timeout=HTTP_TIMEOUT,
        )
        assert setup_resp is not None and "setup-ok" in setup_resp.get("stdout", ""), (
            f"VM {name} never became exec-ready after provision: {setup_resp}"
        )

        # 2. Stop it.
        client.post(f"/stop/{name}", {})

        # 3. Resume -- returns immediately, process not yet listening.
        resume_resp = client.post(f"/resume/{name}", {})
        assert resume_resp is not None, "resume failed"

        # 4. Immediately exec -- no wait_exec_ready, no sleep.
        exec_resp = client.post(
            f"/exec/{name}",
            {"command": "echo resumed-no-wait", "timeout_secs": EXEC_TIMEOUT_SECS},
            timeout=HTTP_TIMEOUT,
        )
        assert exec_resp is not None, "exec after resume returned None"
        assert "resumed-no-wait" in exec_resp.get("stdout", ""), (
            f"expected 'resumed-no-wait' in stdout, got: {exec_resp}"
        )
        assert exec_resp.get("exit_code") == 0

        client.delete(f"/delete/{name}")
