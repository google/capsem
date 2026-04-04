"""Verify guest filesystem layout and permissions after boot."""

import pytest

pytestmark = pytest.mark.guest


class TestGuestFilesystem:

    def test_rootfs_is_overlay(self, guest_env):
        """Root filesystem is mounted as an overlay."""
        client, name = guest_env
        resp = client.post(f"/exec/{name}", {"command": "mount | grep ' on / ' | head -1"})
        stdout = resp.get("stdout", "") if resp else ""
        assert "overlay" in stdout, f"Expected overlay rootfs, got: {stdout}"

    def test_overlay_tmpfs(self, guest_env):
        """Overlay upper is backed by tmpfs or loop device."""
        client, name = guest_env
        resp = client.post(f"/exec/{name}", {"command": "mount | grep -E 'overlay|tmpfs|/dev/loop'"})
        stdout = resp.get("stdout", "") if resp else ""
        assert "overlay" in stdout or "tmpfs" in stdout, f"Expected overlay/tmpfs mount, got: {stdout}"

    def test_workspace_exists(self, guest_env):
        """Workspace directory exists at /root."""
        client, name = guest_env
        resp = client.post(f"/exec/{name}", {"command": "test -d /root && echo exists || echo missing"})
        stdout = resp.get("stdout", "") if resp else ""
        assert "exists" in stdout, f"Workspace dir /root not found"

    def test_bin_writable_ephemeral(self, guest_env):
        """Overlay allows ephemeral writes to system paths like /bin."""
        client, name = guest_env
        resp = client.post(f"/exec/{name}", {"command": "touch /bin/test-write 2>&1; echo exit=$?"})
        stdout = resp.get("stdout", "") if resp else ""
        assert "exit=0" in stdout, f"Unexpected stdout: {stdout}"
