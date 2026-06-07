"""Service startup and connectivity tests.

These tests verify that capsem-service actually starts, binds its socket,
and accepts connections -- the exact failure mode that was missed when
'just test' excluded integration tests.
"""

import socket

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import ServiceInstance, wait_exec_ready, vm_name

pytestmark = pytest.mark.integration


class TestServiceStartup:

    def test_socket_accepts_connections(self, service_env):
        """Service socket must accept TCP connections, not just exist on disk."""
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            sock.connect(str(service_env.uds_path))
        except ConnectionRefusedError:
            pytest.fail(
                f"Service socket exists at {service_env.uds_path} "
                "but refuses connections"
            )
        finally:
            sock.close()

    def test_list_endpoint_responds(self, client):
        """The /list endpoint must respond (proves Axum routing works)."""
        resp = client.get("/list")
        assert resp is not None, "/list returned empty response"
        assert isinstance(resp, (dict, list)), f"Unexpected /list response: {resp}"

    def test_provision_creates_vm_socket(self, client):
        """Provisioning a VM must create a per-VM socket that accepts connections."""
        name = vm_name("startup")
        resp = client.post("/provision", {"name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        try:
            assert resp is not None, "Provision returned empty response"
            vm_id = resp.get("id", name)
            assert wait_exec_ready(client, vm_id, timeout=EXEC_READY_TIMEOUT), (
                f"VM {vm_id} never became exec-ready -- "
                "service->process->VM boot chain is broken"
            )
        finally:
            try:
                client.delete(f"/delete/{name}")
            except Exception:
                pass

    def test_service_clean_shutdown(self):
        """Service must shut down cleanly without orphaning processes."""
        svc = ServiceInstance()
        svc.start()
        proc = svc.proc
        pid = proc.pid
        svc.stop()
        assert proc.returncode is not None, (
            f"Service process {pid} did not terminate after stop()"
        )

    def test_shutdown_kills_vm_processes(self):
        """SIGTERM on capsem-service must also kill all per-VM capsem-process children.

        Regression test for a leak where graceful_shutdown killed the gateway +
        tray companions but left running VMs orphaned; every test-suite run
        accumulated ghosts that held Apple VZ memory and caused subsequent
        boot timeouts.
        """
        import os
        import signal
        import time

        svc = ServiceInstance()
        svc.start()
        try:
            client = svc.client()
            name = vm_name("shut")
            resp = client.post("/provision", {
                "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
            })
            assert resp is not None
            assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT), (
                f"VM {name} never exec-ready"
            )

            info = client.get(f"/info/{name}")
            vm_pid = info.get("pid")
            assert vm_pid and vm_pid > 0, f"no pid in /info response: {info}"
        finally:
            svc.stop()

        # After svc.stop() the per-VM capsem-process must also be dead.
        # Poll briefly -- reaping can race by a few ms.
        deadline = time.time() + 3
        while time.time() < deadline:
            try:
                os.kill(vm_pid, 0)
            except ProcessLookupError:
                return  # dead, as expected
            time.sleep(0.05)
        pytest.fail(
            f"capsem-process pid {vm_pid} survived service shutdown -- leak"
        )
