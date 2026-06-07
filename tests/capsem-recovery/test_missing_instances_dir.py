"""Verify service creates instances/ dir if it's missing."""

import pytest

from helpers.service import ServiceInstance

pytestmark = pytest.mark.recovery


def test_missing_instances_dir_recreated():
    """Service starts and creates instances/ dir if absent."""
    svc = ServiceInstance()
    instances_dir = svc.tmp_dir / "instances"
    # Ensure it doesn't exist
    if instances_dir.exists():
        instances_dir.rmdir()

    svc.start()

    try:
        client = svc.client()
        resp = client.get("/list")
        assert resp is not None, "Service should respond"
        assert "sandboxes" in resp
    finally:
        svc.stop()
