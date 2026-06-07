"""Persistence lifecycle tests: create, touch file, stop, resume, verify file survives.

Tests the full persistent VM lifecycle:
- Named VMs are persistent, unnamed are ephemeral
- Persistent VMs survive stop + resume (workspace files persist)
- Creating a VM with an existing name is rejected (must use resume)
- Stop endpoint preserves persistent state but destroys ephemeral state
- Purge kills ephemeral VMs but not persistent ones (unless --all)
- The /run endpoint provisions, execs, and destroys in one shot
"""

import uuid

import pytest

from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB, EXEC_READY_TIMEOUT, EXEC_TIMEOUT_SECS
from helpers.service import wait_exec_ready, vm_name

pytestmark = pytest.mark.integration


class TestPersistentCreate:

    def test_named_vm_is_persistent(self, client):
        """Named VMs should have persistent=true in info."""
        name = vm_name("pers")
        resp = client.post("/vms/create", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True,
        })
        assert resp is not None
        try:
            info = client.get(f"/vms/{name}/info")
            assert info["persistent"] is True
        finally:
            client.delete(f"/vms/{name}/delete")

    def test_unnamed_vm_is_ephemeral(self, client):
        """Unnamed VMs should have persistent=false."""
        resp = client.post("/vms/create", {"ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        vm_id = resp["id"]
        try:
            info = client.get(f"/vms/{vm_id}/info")
            assert info["persistent"] is False
        finally:
            client.delete(f"/vms/{vm_id}/delete")

    def test_create_duplicate_persistent_rejected(self, client):
        """Creating a persistent VM with an existing name must fail."""
        name = vm_name("dup")
        client.post("/vms/create", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True,
        })
        try:
            resp = client.post("/vms/create", {
                "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True,
            })
            assert resp is None or "error" in str(resp).lower() or "already exists" in str(resp).lower(), (
                f"Expected error for duplicate persistent name, got: {resp}"
            )
        finally:
            client.delete(f"/vms/{name}/delete")


class TestStopSemantics:

    def test_stop_persistent_preserves_in_list(self, client):
        """Stopping a persistent VM should keep it in list as Stopped."""
        name = vm_name("stp")
        client.post("/vms/create", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True,
        })
        wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)
        client.post(f"/vms/{name}/stop", {})

        listing = client.get("/vms/list")
        vm = next((s for s in listing["sandboxes"] if s["id"] == name), None)
        assert vm is not None, f"Persistent VM {name} not in list after stop"
        assert vm["status"] == "Stopped"
        assert vm["persistent"] is True

        # Cleanup
        client.delete(f"/vms/{name}/delete")

    def test_stop_ephemeral_removes_from_list(self, client):
        """Stopping an ephemeral VM should destroy it completely."""
        resp = client.post("/vms/create", {"ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        vm_id = resp["id"]
        wait_exec_ready(client, vm_id, timeout=EXEC_READY_TIMEOUT)
        client.post(f"/vms/{vm_id}/stop", {})

        listing = client.get("/vms/list")
        ids = [s["id"] for s in listing["sandboxes"]]
        assert vm_id not in ids, f"Ephemeral VM {vm_id} still in list after stop"


class TestResumeLifecycle:

    def test_create_stop_resume_file_survives(self, client):
        """The core persistence test: create VM, write file, stop, resume, read file back."""
        name = vm_name("life")
        # 1. Create persistent VM
        client.post("/vms/create", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True,
        })
        wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

        # 2. Write a file inside the VM
        marker = f"persistence-test-{uuid.uuid4().hex[:8]}"
        client.post(f"/vms/{name}/files/write", {
            "path": f"/root/{marker}",
            "content": f"hello from {marker}",
        })

        # 3. Verify file exists
        read_resp = client.post(f"/vms/{name}/files/read", {"path": f"/root/{marker}"})
        assert marker in str(read_resp), f"File not found before stop: {read_resp}"

        # 4. Stop the VM (preserves state)
        client.post(f"/vms/{name}/stop", {})

        # 5. Resume
        resume_resp = client.post(f"/vms/{name}/resume", {})
        assert resume_resp is not None
        resumed_id = resume_resp.get("id", name)
        wait_exec_ready(client, resumed_id, timeout=EXEC_READY_TIMEOUT)

        # 6. Read the file back -- it must survive
        read_resp2 = client.post(f"/vms/{resumed_id}/files/read", {"path": f"/root/{marker}"})
        assert marker in str(read_resp2), (
            f"File did not survive stop+resume! Before: had marker. After: {read_resp2}"
        )

        # Cleanup
        client.delete(f"/vms/{resumed_id}/delete")

    def test_resume_nonexistent_fails(self, client):
        """Resuming a VM that doesn't exist should fail."""
        resp = client.post("/vms/no-such-vm-xyz/resume", {})
        assert resp is None or "error" in str(resp).lower()

    def test_resume_running_returns_id(self, client):
        """Resuming an already-running persistent VM should return its ID."""
        name = vm_name("runres")
        client.post("/vms/create", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True,
        })
        wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)

        # Resume while running
        resp = client.post(f"/vms/{name}/resume", {})
        assert resp is not None
        assert resp.get("id") == name

        client.delete(f"/vms/{name}/delete")


class TestPersistConvert:

    def test_persist_converts_ephemeral(self, client):
        """The persist endpoint should convert an ephemeral VM to persistent."""
        resp = client.post("/vms/create", {"ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        vm_id = resp["id"]
        wait_exec_ready(client, vm_id, timeout=EXEC_READY_TIMEOUT)

        new_name = vm_name("conv")
        persist_resp = client.post(f"/vms/{vm_id}/save", {"name": new_name})
        assert persist_resp is not None
        assert "success" in str(persist_resp).lower() or new_name in str(persist_resp)

        # Verify it shows as persistent
        info = client.get(f"/vms/{new_name}/info")
        assert info is not None
        assert info["persistent"] is True

        client.delete(f"/vms/{new_name}/delete")

    def test_persist_rejects_duplicate_name(self, client):
        """Converting to a name that already exists should fail."""
        # Create a persistent VM with a name
        taken = vm_name("taken")
        client.post("/vms/create", {
            "name": taken, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True,
        })

        # Create an ephemeral VM
        resp = client.post("/vms/create", {"ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        vm_id = resp["id"]

        try:
            # Try to persist with the taken name
            persist_resp = client.post(f"/vms/{vm_id}/save", {"name": taken})
            assert persist_resp is None or "error" in str(persist_resp).lower()
        finally:
            client.delete(f"/vms/{vm_id}/delete")
            client.delete(f"/vms/{taken}/delete")


class TestPurge:

    def test_purge_kills_ephemeral_only(self, client):
        """Purge without --all should only kill ephemeral VMs."""
        persistent_name = vm_name("pkeep")
        client.post("/vms/create", {
            "name": persistent_name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True,
        })
        eph_resp = client.post("/vms/create", {"ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS})
        eph_id = eph_resp["id"]

        purge_resp = client.post("/purge", {"all": False})
        assert purge_resp is not None

        listing = client.get("/vms/list")
        ids = [s["id"] for s in listing["sandboxes"]]
        assert persistent_name in ids, "Persistent VM was killed by purge without --all"
        assert eph_id not in ids, "Ephemeral VM survived purge"

        client.delete(f"/vms/{persistent_name}/delete")

    def test_purge_all_destroys_persistent(self, client):
        """Purge with all=true should destroy persistent VMs too."""
        persistent_name = vm_name("pall")
        client.post("/vms/create", {
            "name": persistent_name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True,
        })

        purge_resp = client.post("/purge", {"all": True})
        assert purge_resp is not None
        assert purge_resp.get("persistent_purged", 0) >= 1

        listing = client.get("/vms/list")
        ids = [s["id"] for s in listing["sandboxes"]]
        assert persistent_name not in ids, "Persistent VM survived purge --all"

    def test_purge_default_all_is_false(self, client):
        """Purge with empty body defaults all=false (safe default)."""
        persistent_name = vm_name("pdef")
        client.post("/vms/create", {
            "name": persistent_name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True,
        })

        # Empty body -- all should default to false
        purge_resp = client.post("/purge", {})
        assert purge_resp is not None

        listing = client.get("/vms/list")
        ids = [s["id"] for s in listing["sandboxes"]]
        assert persistent_name in ids, "Persistent VM was killed by purge with default all=false"

        client.delete(f"/vms/{persistent_name}/delete")


class TestRunEndpoint:

    def test_run_returns_output(self, client):
        """The /run endpoint should exec a command and return output."""
        resp = client.post("/run", {
            "command": "echo hello-from-run",
            "timeout_secs": EXEC_TIMEOUT_SECS,
        })
        assert resp is not None
        assert "hello-from-run" in resp.get("stdout", ""), f"Unexpected response: {resp}"
        assert resp.get("exit_code") == 0

    def test_run_nonzero_exit(self, client):
        """The /run endpoint should propagate non-zero exit codes."""
        resp = client.post("/run", {
            "command": "exit 42",
            "timeout_secs": EXEC_TIMEOUT_SECS,
        })
        assert resp is not None
        assert resp.get("exit_code") == 42


class TestListPersistence:

    def test_list_shows_stopped_persistent(self, client):
        """Stopped persistent VMs should appear in list with status Stopped."""
        name = vm_name("lstp")
        client.post("/vms/create", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True,
        })
        wait_exec_ready(client, name, timeout=EXEC_READY_TIMEOUT)
        client.post(f"/vms/{name}/stop", {})

        listing = client.get("/vms/list")
        vm = next((s for s in listing["sandboxes"] if s["id"] == name), None)
        assert vm is not None, "Stopped persistent VM not in list"
        assert vm["status"] == "Stopped"
        assert vm["pid"] == 0

        client.delete(f"/vms/{name}/delete")

    def test_list_persistent_field(self, client):
        """List should include the persistent field for all VMs."""
        name = vm_name("lpf")
        client.post("/vms/create", {
            "name": name, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, "persistent": True,
        })
        try:
            listing = client.get("/vms/list")
            vm = next((s for s in listing["sandboxes"] if s["id"] == name), None)
            assert vm is not None
            assert "persistent" in vm
            assert vm["persistent"] is True
        finally:
            client.delete(f"/vms/{name}/delete")
