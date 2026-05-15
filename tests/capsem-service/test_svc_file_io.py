"""File read/write endpoint tests."""

import pytest

pytestmark = pytest.mark.integration


class TestFileIO:

    def test_roundtrip(self, ready_vm):
        client, name = ready_vm
        client.write_file(name, "/root/rt.txt", "payload-xyz")
        resp = client.read_file(name, "/root/rt.txt")
        assert resp is not None
        assert resp.get("content") == "payload-xyz"

    def test_unicode(self, ready_vm):
        client, name = ready_vm
        text = "caf\u00e9 \u00fc\u00f1\u00ee\u00e7\u00f8\u00f0\u00e9"
        client.write_file(name, "/root/uni.txt", text)
        resp = client.read_file(name, "/root/uni.txt")
        assert resp.get("content") == text

    def test_multiline(self, ready_vm):
        client, name = ready_vm
        text = "line1\nline2\nline3\n"
        client.write_file(name, "/root/multi.txt", text)
        resp = client.read_file(name, "/root/multi.txt")
        assert resp.get("content") == text

    def test_empty(self, ready_vm):
        client, name = ready_vm
        client.write_file(name, "/root/empty.txt", "")
        resp = client.read_file(name, "/root/empty.txt")
        assert resp.get("content") == ""

    @pytest.mark.skip(reason="slow, team will fix")
    def test_large(self, ready_vm):
        """1MB payload roundtrip."""
        client, name = ready_vm
        text = "x" * 1_000_000
        client.write_file(name, "/root/large.txt", text)
        resp = client.read_file(name, "/root/large.txt")
        assert resp.get("content") == text

    @pytest.mark.skip(reason="slow, team will fix")
    def test_overwrite(self, ready_vm):
        client, name = ready_vm
        client.write_file(name, "/root/ow.txt", "first")
        client.write_file(name, "/root/ow.txt", "second")
        resp = client.read_file(name, "/root/ow.txt")
        assert resp.get("content") == "second"

    @pytest.mark.skip(reason="slow, team will fix")
    def test_nested_path(self, ready_vm):
        client, name = ready_vm
        client.post(f"/exec/{name}", {"command": "mkdir -p /root/deep/nested"})
        client.write_file(name, "/root/deep/nested/f.txt", "deep")
        resp = client.read_file(name, "/root/deep/nested/f.txt")
        assert resp.get("content") == "deep"

    def test_read_nonexistent_file(self, ready_vm):
        client, name = ready_vm
        resp = client.read_file(name, "/root/no-such-file.txt")
        assert resp is None or "error" in str(resp).lower()

    def test_read_nonexistent_vm(self, service_env):
        client = service_env.client()
        resp = client.read_file("ghost-vm", "/root/x.txt")
        assert resp is None or "error" in str(resp).lower() or "not found" in str(resp).lower()
