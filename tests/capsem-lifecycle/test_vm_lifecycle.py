"""VM lifecycle integration tests: shutdown, suspend/resume, identity.

Tests the full guest-initiated lifecycle:
- Guest shutdown via /sbin/shutdown stops ephemeral and persistent VMs
- Persistent VM survives guest shutdown + resume with file persistence
- CAPSEM_VM_ID and CAPSEM_VM_NAME env vars are injected
- Hostname reflects the VM name
- Suspend + warm resume round-trip (Apple VZ)
"""

import time
import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT
from helpers.service import wait_exec_ready, vm_name

pytestmark = pytest.mark.integration


class TestGuestShutdownEphemeral:

    def test_guest_shutdown_stops_ephemeral(self, client):
        """Typing 'shutdown' inside an ephemeral VM should stop it."""
        resp = client.post("/provision", {"ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        vm_id = resp["id"]
        assert wait_exec_ready(client, vm_id, timeout=EXEC_READY_TIMEOUT), \
            f"VM {vm_id} never became exec-ready"

        # Trigger guest-initiated shutdown (capsem-sysutil sends ShutdownRequest).
        # Use nohup so the exec doesn't block waiting for shutdown to complete.
        # The countdown is ~4s (SHUTDOWN_GRACE_SECS + 1), so we fire-and-forget.
        client.post(f"/exec/{vm_id}", {
            "command": "nohup /run/capsem-sysutil shutdown </dev/null >/dev/null 2>&1 &",
        })

        # Wait for VM to disappear from list (service reaps ephemeral on exit)
        gone = False
        for _ in range(20):
            time.sleep(1)
            listing = client.get("/list")
            ids = [s["id"] for s in listing["sandboxes"]]
            if vm_id not in ids:
                gone = True
                break
        assert gone, f"Ephemeral VM {vm_id} still in list after guest shutdown"


class TestGuestShutdownPersistent:

    def test_guest_shutdown_preserves_persistent_and_resume(self, client):
        """Guest shutdown on a persistent VM preserves state; resume restores it."""
        name = vm_name("gshut")
        client.post("/provision", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True,
        })
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT), \
            f"VM {name} never became exec-ready"

        # Write a marker file
        marker = f"shutdown-test-{uuid.uuid4().hex[:8]}"
        client.post(f"/write_file/{name}", {
            "path": f"/root/{marker}",
            "content": f"hello from {marker}",
        })

        # Guest-initiated shutdown
        client.post(f"/exec/{name}", {
            "command": "nohup /run/capsem-sysutil shutdown </dev/null >/dev/null 2>&1 &",
        })

        # Wait for VM to stop
        stopped = False
        for _ in range(20):
            time.sleep(1)
            listing = client.get("/list")
            vm = next((s for s in listing["sandboxes"] if s["id"] == name), None)
            if vm and vm["status"] == "Stopped":
                stopped = True
                break
            if vm is None:
                # Might have been removed from running list but still in registry
                try:
                    info = client.get(f"/info/{name}")
                    if info and info.get("status") == "Stopped":
                        stopped = True
                        break
                except Exception:
                    pass
        assert stopped, f"Persistent VM {name} did not reach Stopped after guest shutdown"

        # Resume and verify file survived
        resume_resp = client.post(f"/resume/{name}", {})
        assert resume_resp is not None
        resumed_id = resume_resp.get("id", name)
        assert wait_exec_ready(client, resumed_id, timeout=EXEC_READY_TIMEOUT), \
            f"VM {resumed_id} never became exec-ready after resume"

        read_resp = client.post(f"/read_file/{resumed_id}", {"path": f"/root/{marker}"})
        assert isinstance(read_resp, dict) and "content" in read_resp, \
            f"read_file returned an error instead of content: {read_resp}"
        assert marker in read_resp["content"], \
            f"File did not survive guest shutdown + resume: {read_resp}"

        client.delete(f"/delete/{resumed_id}")


class TestVmIdentity:

    def test_capsem_vm_id_env_var(self, client):
        """CAPSEM_VM_ID must be set inside the VM."""
        name = vm_name("vmid")
        client.post("/provision", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True,
        })
        try:
            assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)
            resp = client.post(f"/exec/{name}", {"command": "echo $CAPSEM_VM_ID"})
            vm_id = resp["stdout"].strip()
            assert vm_id, "CAPSEM_VM_ID is empty"
            assert len(vm_id) > 0
        finally:
            client.delete(f"/delete/{name}")

    def test_capsem_vm_name_env_var(self, client):
        """CAPSEM_VM_NAME must be set to the VM name for persistent VMs."""
        name = vm_name("vmname")
        client.post("/provision", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True,
        })
        try:
            assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)
            resp = client.post(f"/exec/{name}", {"command": "echo $CAPSEM_VM_NAME"})
            vm_name_val = resp["stdout"].strip()
            assert vm_name_val == name, \
                f"CAPSEM_VM_NAME={vm_name_val!r}, expected {name!r}"
        finally:
            client.delete(f"/delete/{name}")

    def test_hostname_reflects_vm_name(self, client):
        """Hostname inside the VM must match the VM name."""
        name = vm_name("hname")
        client.post("/provision", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True,
        })
        try:
            assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)
            resp = client.post(f"/exec/{name}", {"command": "hostname"})
            hostname = resp["stdout"].strip()
            assert hostname == name, \
                f"hostname={hostname!r}, expected {name!r}"
        finally:
            client.delete(f"/delete/{name}")

    def test_ephemeral_vm_has_id_as_hostname(self, client):
        """Ephemeral VMs should get CAPSEM_VM_ID as hostname."""
        resp = client.post("/provision", {"ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        vm_id = resp["id"]
        try:
            assert wait_exec_ready(client, vm_id, timeout=EXEC_READY_TIMEOUT)
            id_resp = client.post(f"/exec/{vm_id}", {"command": "echo $CAPSEM_VM_ID"})
            hostname_resp = client.post(f"/exec/{vm_id}", {"command": "hostname"})
            capsem_id = id_resp["stdout"].strip()
            hostname = hostname_resp["stdout"].strip()
            assert capsem_id, "CAPSEM_VM_ID not set for ephemeral VM"
            assert hostname == capsem_id, \
                f"ephemeral hostname={hostname!r} != CAPSEM_VM_ID={capsem_id!r}"
        finally:
            client.delete(f"/delete/{vm_id}")


class TestStopResumeE2E:

    def test_file_survives_stop_resume(self, client):
        """E2E: create -> write file -> stop -> resume -> read file -> delete."""
        name = vm_name("e2efile")
        client.post("/provision", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True,
        })
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

        marker = f"e2e-{uuid.uuid4().hex[:8]}"
        client.post(f"/write_file/{name}", {
            "path": f"/root/{marker}",
            "content": f"hello from {marker}",
        })

        # Stop
        client.post(f"/stop/{name}", {})

        # Resume
        resume_resp = client.post(f"/resume/{name}", {})
        assert resume_resp is not None
        resumed_id = resume_resp.get("id", name)
        assert wait_exec_ready(client, resumed_id, timeout=EXEC_READY_TIMEOUT)

        # Read back
        read_resp = client.post(f"/read_file/{resumed_id}", {"path": f"/root/{marker}"})
        assert marker in str(read_resp), \
            f"File did not survive stop + resume: {read_resp}"

        client.delete(f"/delete/{resumed_id}")

    def test_env_survives_stop_resume(self, client):
        """E2E: create with env -> stop -> resume -> verify env -> delete."""
        name = vm_name("e2eenv")
        env_key = "CAPSEM_E2E_TEST"
        env_val = f"lifecycle-{uuid.uuid4().hex[:8]}"
        client.post("/provision", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True,
            "env": {env_key: env_val},
        })
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

        # Verify env is set
        resp = client.post(f"/exec/{name}", {"command": f"echo ${env_key}"})
        assert env_val in resp["stdout"], \
            f"{env_key} not set before stop: {resp['stdout']}"

        # Stop
        client.post(f"/stop/{name}", {})

        # Resume
        resume_resp = client.post(f"/resume/{name}", {})
        assert resume_resp is not None
        resumed_id = resume_resp.get("id", name)
        assert wait_exec_ready(client, resumed_id, timeout=EXEC_READY_TIMEOUT)

        # Verify env survives
        resp2 = client.post(f"/exec/{resumed_id}", {"command": f"echo ${env_key}"})
        assert env_val in resp2["stdout"], \
            f"{env_key} did not survive stop + resume: {resp2['stdout']}"

        client.delete(f"/delete/{resumed_id}")


class TestSuspendResume:

    def test_suspend_resume_round_trip(self, client):
        """Suspend a persistent VM, resume it, verify file survives."""
        name = vm_name("susp")
        client.post("/provision", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True,
        })
        assert wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT), \
            f"VM {name} never became exec-ready"

        # Write a marker file
        marker = f"suspend-test-{uuid.uuid4().hex[:8]}"
        client.post(f"/write_file/{name}", {
            "path": f"/root/{marker}",
            "content": f"hello from {marker}",
        })

        # Suspend via service API
        suspend_resp = client.post(f"/suspend/{name}", {}, timeout=EXEC_READY_TIMEOUT)
        assert suspend_resp is not None and suspend_resp.get("success") is True, \
            f"Suspend failed: {suspend_resp}"

        # Verify VM shows as Suspended
        listing = client.get("/list")
        vm = next((s for s in listing["sandboxes"] if s["id"] == name), None)
        assert vm is not None, f"Suspended VM {name} not in list"
        assert vm["status"] == "Suspended", f"Expected Suspended, got {vm['status']}"

        # Resume (warm restore)
        resume_resp = client.post(f"/resume/{name}", {})
        assert resume_resp is not None
        resumed_id = resume_resp.get("id", name)
        assert wait_exec_ready(client, resumed_id, timeout=EXEC_READY_TIMEOUT), \
            f"VM {resumed_id} never became exec-ready after warm resume"

        # Verify file survived
        read_resp = client.post(f"/read_file/{resumed_id}", {"path": f"/root/{marker}"})
        assert marker in str(read_resp), \
            f"File did not survive suspend + resume: {read_resp}"

        client.delete(f"/delete/{resumed_id}")

    def test_suspend_ephemeral_rejected(self, client):
        """Suspending an ephemeral VM must fail."""
        resp = client.post("/provision", {"ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        vm_id = resp["id"]
        try:
            assert wait_exec_ready(client, vm_id, timeout=EXEC_READY_TIMEOUT)
            suspend_resp = client.post(f"/suspend/{vm_id}", {})
            # Should fail (400 or error in response)
            assert suspend_resp is None or "error" in str(suspend_resp).lower() \
                or "cannot" in str(suspend_resp).lower(), \
                f"Expected error for ephemeral suspend, got: {suspend_resp}"
        finally:
            client.delete(f"/delete/{vm_id}")
