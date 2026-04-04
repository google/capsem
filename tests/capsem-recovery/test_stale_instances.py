"""Verify service handles stale instance sockets on startup."""

import os
import uuid

import pytest

from pathlib import Path

from helpers.service import ServiceInstance

pytestmark = pytest.mark.recovery


def test_stale_instance_sockets():
    """Service starts with stale .sock files in instances/ directory."""
    svc = ServiceInstance()

    # Create fake instance sockets
    instances_dir = svc.tmp_dir / "instances"
    instances_dir.mkdir(parents=True, exist_ok=True)
    for i in range(3):
        fake_sock = instances_dir / f"stale-{i}-{uuid.uuid4().hex[:6]}.sock"
        fake_sock.touch()

    svc.start()

    try:
        client = svc.client()
        resp = client.get("/list")
        assert resp is not None, "Service should start despite stale instance sockets"
    finally:
        svc.stop()
