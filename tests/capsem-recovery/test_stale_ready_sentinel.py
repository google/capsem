"""Verify service handles stale .ready sentinel files from crashed VMs."""

import uuid

import pytest

from helpers.service import ServiceInstance

pytestmark = pytest.mark.recovery


def test_stale_ready_sentinels_ignored():
    """Stale .ready files in instances/ don't confuse service."""
    svc = ServiceInstance()
    instances_dir = svc.tmp_dir / "instances"
    instances_dir.mkdir(parents=True, exist_ok=True)

    # Create fake .ready sentinels (left behind by crashed capsem-process)
    for i in range(3):
        sentinel = instances_dir / f"stale-{i}-{uuid.uuid4().hex[:6]}.ready"
        sentinel.touch()

    svc.start()

    try:
        client = svc.client()
        resp = client.get("/list")
        assert resp is not None, "Service should start despite stale sentinels"
        # Stale sentinels should not appear as running VMs
        ids = [s["id"] for s in resp.get("sandboxes", [])]
        stale_ids = [i for i in ids if i.startswith("stale-")]
        assert len(stale_ids) == 0, f"Stale sentinels appeared as VMs: {stale_ids}"
    finally:
        svc.stop()
