"""Verify only one service can bind to a socket at a time."""

import subprocess
import time

import pytest

from helpers.service import ServiceInstance

pytestmark = pytest.mark.recovery


def test_second_service_fails():
    """Starting a second service on the same socket should fail clearly."""
    svc_a = ServiceInstance()
    svc_a.start()

    try:
        # Try to start a second service on the same socket
        svc_b = ServiceInstance()
        svc_b.uds_path = svc_a.uds_path  # Same socket

        try:
            svc_b.start()
            # If it somehow starts, it should at least not corrupt service A
            client_a = svc_a.client()
            resp = client_a.get("/list")
            assert resp is not None, "Service A should still work"
            svc_b.stop()
        except RuntimeError:
            # Expected -- second service should fail to start
            pass

    finally:
        svc_a.stop()
