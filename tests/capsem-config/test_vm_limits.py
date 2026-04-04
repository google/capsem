"""VM count limit enforcement."""

import uuid

import pytest

from helpers.service import ServiceInstance

pytestmark = pytest.mark.config

# NOTE: The max_concurrent_vms config key does not exist yet.
# This test documents the requirement. See implementation-tasks.md.
# Until implemented, these tests will fail -- that's intentional (TDD).


def test_provision_at_limit_rejected():
    """Provisioning beyond max_concurrent_vms should fail with a clear error."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    created = []

    try:
        # Default limit should be 10 (per implementation-tasks.md)
        max_vms = 10
        for i in range(max_vms):
            name = f"limit-{i}-{uuid.uuid4().hex[:6]}"
            resp = client.post("/provision", {"name": name, "ram_mb": 512, "cpus": 1})
            assert resp is not None and "id" in str(resp), f"VM {i} should succeed: {resp}"
            created.append(name)

        # VM #11 should be rejected
        name = f"limit-over-{uuid.uuid4().hex[:6]}"
        resp = client.post("/provision", {"name": name, "ram_mb": 512, "cpus": 1})
        # Should be rejected
        assert resp is None or "error" in str(resp).lower() or "limit" in str(resp).lower() or "maximum" in str(resp).lower(), (
            f"Expected limit error, got: {resp}"
        )

    finally:
        for vm_id in created:
            try:
                client.delete(f"/delete/{vm_id}")
            except Exception:
                pass
        svc.stop()


def test_delete_frees_slot():
    """After deleting a VM, a new one can be created within the limit."""
    svc = ServiceInstance()
    svc.start()
    client = svc.client()
    created = []

    try:
        # Fill to limit
        max_vms = 10
        for i in range(max_vms):
            name = f"slot-{i}-{uuid.uuid4().hex[:6]}"
            client.post("/provision", {"name": name, "ram_mb": 512, "cpus": 1})
            created.append(name)

        # Delete one
        deleted = created.pop()
        client.delete(f"/delete/{deleted}")

        # Should be able to create one more
        name = f"slot-new-{uuid.uuid4().hex[:6]}"
        resp = client.post("/provision", {"name": name, "ram_mb": 512, "cpus": 1})
        assert resp is not None and "error" not in str(resp).lower(), (
            f"Should succeed after freeing a slot: {resp}"
        )
        created.append(name)

    finally:
        for vm_id in created:
            try:
                client.delete(f"/delete/{vm_id}")
            except Exception:
                pass
        svc.stop()
