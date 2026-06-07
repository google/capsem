"""Verify service handles partial session directories from failed boots."""

import uuid

import pytest

from helpers.service import ServiceInstance

pytestmark = pytest.mark.recovery


def test_partial_session_dir_handled():
    """Incomplete session dir (no session.db) from failed boot doesn't crash service."""
    svc = ServiceInstance()

    # Create an incomplete session directory (as if boot failed mid-setup)
    sessions_dir = svc.tmp_dir / "sessions"
    sessions_dir.mkdir(parents=True, exist_ok=True)
    partial_dir = sessions_dir / f"partial-{uuid.uuid4().hex[:8]}"
    partial_dir.mkdir()
    # Write a workspace dir but no session.db
    (partial_dir / "workspace").mkdir()

    svc.start()

    try:
        client = svc.client()
        resp = client.get("/list")
        assert resp is not None, "Service should start despite partial session dir"
    finally:
        svc.stop()


def test_empty_session_dir_handled():
    """Empty session dir doesn't prevent service startup."""
    svc = ServiceInstance()

    sessions_dir = svc.tmp_dir / "sessions"
    sessions_dir.mkdir(parents=True, exist_ok=True)
    empty_dir = sessions_dir / f"empty-{uuid.uuid4().hex[:8]}"
    empty_dir.mkdir()

    svc.start()

    try:
        client = svc.client()
        resp = client.get("/list")
        assert resp is not None
    finally:
        svc.stop()
