"""Provision, list, info, and delete endpoint tests."""

import uuid

import pytest
from helpers.constants import CODE_PROFILE_ID, DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import vm_name

pytestmark = pytest.mark.integration


class TestProvision:

    def test_create_with_name(self, fresh_vm):
        name, resp = fresh_vm("prov")
        assert resp is not None
        assert uuid.UUID(resp["id"])
        assert resp["name"] == name

    def test_create_without_name(self, client):
        resp = client.post(
            "/vms/create",
            {"profile_id": CODE_PROFILE_ID, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS},
        )
        assert resp is not None
        vm_id = resp.get("id")
        assert vm_id, f"No ID in response: {resp}"
        client.delete(f"/vms/{vm_id}/delete")

    def test_session_name_create_without_name_uses_profile_counter(self, client):
        first = client.post(
            "/vms/create",
            {"profile_id": CODE_PROFILE_ID, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS},
        )
        second = client.post(
            "/vms/create",
            {"profile_id": CODE_PROFILE_ID, "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS},
        )
        first_id = first.get("id")
        second_id = second.get("id")
        profile_prefix = f"{CODE_PROFILE_ID}-"
        try:
            assert uuid.UUID(first_id)
            assert uuid.UUID(second_id)
            assert first["name"].startswith(profile_prefix)
            assert second["name"].startswith(profile_prefix)
            first_num = int(first["name"].removeprefix(profile_prefix))
            second_num = int(second["name"].removeprefix(profile_prefix))
            assert second_num == first_num + 1
            assert not first["name"].startswith("tmp-")
            assert not second["name"].startswith("tmp-")
        finally:
            if first_id:
                client.delete(f"/vms/{first_id}/delete")
            if second_id:
                client.delete(f"/vms/{second_id}/delete")

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
        assert uuid.UUID(info["id"])
        assert info["id"] == resp["id"]
        assert info["name"] == name

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
        names = [s.get("name") for s in resp["sandboxes"]]
        assert name in names

    def test_list_fields(self, fresh_vm, client):
        name, _ = fresh_vm("fields")
        resp = client.get("/vms/list")
        vm = next(s for s in resp["sandboxes"] if s.get("name") == name)
        assert "id" in vm
        assert uuid.UUID(vm["id"])
        assert vm["name"] == name
        assert "status" in vm


class TestInfo:

    def test_info_valid(self, fresh_vm, client):
        name, _ = fresh_vm("info")
        info = client.get(f"/vms/{name}/info")
        assert info is not None
        assert uuid.UUID(info["id"])
        assert info["name"] == name

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
        # `name`, not `id`: ids are UUIDs, so comparing a name against them
        # passed whether or not the delete did anything.
        names = [s.get("name") for s in resp["sandboxes"]]
        assert name not in names, f"{name} survived delete: {names}"

    def test_delete_twice(self, client):
        """The second delete of one VM is refused.

        Each step is asserted rather than assumed. This test used to run
        create, delete, delete and assert only the last, so a create that had
        not finished registering, or a first delete that had not removed the
        VM, produced a second delete that legitimately succeeded -- and the
        failure named the final assertion rather than the step that actually
        went wrong. It held a binary release attempt saying only
        `assert {'success': True} is None or ...`.
        """
        name = vm_name("del2x")
        created = client.post(
            "/vms/create",
            {
                "name": name,
                "profile_id": CODE_PROFILE_ID,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
            },
        )
        assert created is not None, "create returned nothing"
        assert "error" not in str(created).lower(), f"create failed: {created}"

        # `id` is a UUID; the name lives in `name`. Matching on `id` asserts
        # nothing, because a name is never a UUID -- which is why the
        # neighbouring `test_delete_removes_from_list` passes whether or not
        # the VM was ever removed.
        def names() -> list[str]:
            return [vm.get("name") for vm in client.get("/vms/list")["sandboxes"]]

        assert name in names(), f"{name} is not registered after create: {names()}"

        first = client.delete(f"/vms/{name}/delete")
        assert "error" not in str(first).lower(), f"first delete failed: {first}"

        assert name not in names(), f"{name} still registered after delete: {names()}"

        resp = client.delete(f"/vms/{name}/delete")
        assert resp is None or "error" in str(resp).lower() or "not found" in str(resp).lower(), (
            f"deleting {name} twice succeeded: {resp}"
        )

    def test_delete_nonexistent(self, client):
        resp = client.delete("/vms/no-such-vm-xyz/delete")
        assert resp is None or "error" in str(resp).lower() or "not found" in str(resp).lower()
