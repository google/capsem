"""Verify guest environment variables after boot."""

import pytest

pytestmark = pytest.mark.guest


class TestGuestEnv:

    def test_home_set(self, guest_env):
        """HOME is set to /root."""
        client, name = guest_env
        resp = client.post(f"/exec/{name}", {"command": "echo $HOME"})
        stdout = resp.get("stdout", "").strip() if resp else ""
        assert stdout == "/root", f"Expected HOME=/root, got HOME={stdout}"

    def test_term_set(self, guest_env):
        """TERM environment variable is set."""
        client, name = guest_env
        resp = client.post(f"/exec/{name}", {"command": "echo ${TERM:-unset}"})
        stdout = resp.get("stdout", "").strip() if resp else ""
        assert stdout != "unset", "TERM is not set"

    def test_path_includes_bin(self, guest_env):
        """PATH includes standard binary directories."""
        client, name = guest_env
        resp = client.post(f"/exec/{name}", {"command": "echo $PATH"})
        stdout = resp.get("stdout", "").strip() if resp else ""
        assert "/usr/bin" in stdout or "/bin" in stdout, (
            f"PATH missing standard dirs: {stdout}"
        )

    def test_ld_preload_empty(self, guest_env):
        """LD_PRELOAD is not set (no library injection)."""
        client, name = guest_env
        resp = client.post(f"/exec/{name}", {"command": "echo ${LD_PRELOAD:-empty}"})
        stdout = resp.get("stdout", "").strip() if resp else ""
        assert stdout == "empty", f"LD_PRELOAD should be empty, got: {stdout}"
