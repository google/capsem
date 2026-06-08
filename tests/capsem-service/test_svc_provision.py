"""Provision, list, info, and delete endpoint tests."""

import pytest

from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import vm_name

pytestmark = pytest.mark.integration


class TestProvision:

    def test_create_with_name(self, fresh_vm):
        name, resp = fresh_vm("prov")
        assert resp is not None
        assert resp.get("id") == name or name in str(resp)

    def test_create_without_name(self, client):
        resp = client.post(
            "/vms/create",
            {"profile_id": CODE_PROFILE_ID, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS},
        )
        assert resp is not None
        vm_id = resp.get("id")
        assert vm_id, f"No ID in response: {resp}"
        client.delete(f"/vms/{vm_id}/delete")

    def test_create_with_custom_resources(self, fresh_vm, client):
        name, _ = fresh_vm("res", ram_mb=4096, cpus=4)
        info = client.get(f"/vms/{name}/info")
        assert info is not None
        if "ram_mb" in info:
            assert info["ram_mb"] == 4096
        if "cpus" in info:
            assert info["cpus"] == 4

    def test_create_duplicate_name(self, fresh_vm, client):
        name, _ = fresh_vm("dup")
        # Second create with same name should fail
        resp = client.post(
            "/vms/create",
            {
                "name": name,
                "profile_id": CODE_PROFILE_ID,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
            },
        )
        assert resp is None or "error" in str(resp).lower() or "already" in str(resp).lower(), (
            f"Expected error for duplicate name, got: {resp}"
        )


class TestPersistence:

    def test_provision_persistent(self, fresh_vm, client):
        name, resp = fresh_vm("persist")
        assert resp is not None
        info = client.get(f"/vms/{name}/info")
        assert info is not None
        assert info["id"] == name

    def test_provision_default_not_persistent(self, client):
        resp = client.post(
            "/vms/create",
            {"profile_id": CODE_PROFILE_ID, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS},
        )
        assert resp is not None
        vm_id = resp.get("id")
        assert vm_id
        info = client.get(f"/vms/{vm_id}/info")
        assert info is not None
        # Default VMs are ephemeral (not persistent)
        assert info.get("persistent", False) is False
        client.delete(f"/vms/{vm_id}/delete")


class TestList:

    def test_list_returns_sandboxes(self, client):
        resp = client.get("/vms/list")
        assert resp is not None
        assert "sandboxes" in resp
        assert isinstance(resp["sandboxes"], list)

    def test_list_contains_created_vm(self, fresh_vm, client):
        name, _ = fresh_vm("list")
        resp = client.get("/vms/list")
        ids = [s["id"] for s in resp["sandboxes"]]
        assert name in ids

    def test_list_fields(self, fresh_vm, client):
        name, _ = fresh_vm("fields")
        resp = client.get("/vms/list")
        vm = next(s for s in resp["sandboxes"] if s["id"] == name)
        assert "id" in vm
        assert "status" in vm


class TestInfo:

    def test_info_valid(self, fresh_vm, client):
        name, _ = fresh_vm("info")
        info = client.get(f"/vms/{name}/info")
        assert info is not None
        assert info["id"] == name

    def test_info_nonexistent(self, client):
        resp = client.get("/vms/ghost-vm-404/info")
        assert resp is None or "error" in str(resp).lower() or "not found" in str(resp).lower()


class TestDelete:

    def test_delete_removes_from_list(self, client):
        name = vm_name("del")
        client.post(
            "/vms/create",
            {
                "name": name,
                "profile_id": CODE_PROFILE_ID,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
            },
        )
        client.delete(f"/vms/{name}/delete")
        resp = client.get("/vms/list")
        ids = [s["id"] for s in resp["sandboxes"]]
        assert name not in ids

    def test_delete_twice(self, client):
        name = vm_name("del2x")
        client.post(
            "/vms/create",
            {
                "name": name,
                "profile_id": CODE_PROFILE_ID,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
            },
        )
        client.delete(f"/vms/{name}/delete")
        resp = client.delete(f"/vms/{name}/delete")
        assert resp is None or "error" in str(resp).lower() or "not found" in str(resp).lower()

    def test_delete_nonexistent(self, client):
        resp = client.delete("/vms/no-such-vm-xyz/delete")
        assert resp is None or "error" in str(resp).lower() or "not found" in str(resp).lower()
