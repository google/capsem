"""Verify service starts cleanly when a stale socket exists."""

import os
import tempfile

import pytest

from helpers.service import ServiceInstance

pytestmark = pytest.mark.recovery


def test_stale_socket_replaced():
    """Service replaces a pre-existing stale socket and binds successfully."""
    svc = ServiceInstance()

    # Create a fake stale socket file before starting
    svc.uds_path.parent.mkdir(parents=True, exist_ok=True)
    svc.uds_path.touch()
    assert svc.uds_path.exists(), "Stale socket should exist before start"

    svc.start()

    try:
        client = svc.client()
        resp = client.get("/list")
        assert resp is not None, "Service should respond after replacing stale socket"
        assert "sandboxes" in resp
    finally:
        svc.stop()
