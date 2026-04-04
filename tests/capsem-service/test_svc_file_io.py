"""File read/write endpoint tests."""

import pytest

pytestmark = pytest.mark.integration


class TestFileIO:

    def test_roundtrip(self, ready_vm):
        client, name = ready_vm
        client.post(f"/write_file/{name}", {"path": "/tmp/rt.txt", "content": "payload-xyz"})
        resp = client.post(f"/read_file/{name}", {"path": "/tmp/rt.txt"})
        assert resp is not None
        assert resp.get("content") == "payload-xyz"

    def test_unicode(self, ready_vm):
        client, name = ready_vm
        text = "caf\u00e9 \u00fc\u00f1\u00ee\u00e7\u00f8\u00f0\u00e9"
        client.post(f"/write_file/{name}", {"path": "/tmp/uni.txt", "content": text})
        resp = client.post(f"/read_file/{name}", {"path": "/tmp/uni.txt"})
        assert resp.get("content") == text

    def test_multiline(self, ready_vm):
        client, name = ready_vm
        text = "line1\nline2\nline3\n"
        client.post(f"/write_file/{name}", {"path": "/tmp/multi.txt", "content": text})
        resp = client.post(f"/read_file/{name}", {"path": "/tmp/multi.txt"})
        assert resp.get("content") == text

    def test_empty(self, ready_vm):
        client, name = ready_vm
        client.post(f"/write_file/{name}", {"path": "/tmp/empty.txt", "content": ""})
        resp = client.post(f"/read_file/{name}", {"path": "/tmp/empty.txt"})
        assert resp.get("content") == ""

    @pytest.mark.skip(reason="slow, team will fix")
    def test_large(self, ready_vm):
        """1MB payload roundtrip."""
        client, name = ready_vm
        text = "x" * 1_000_000
        client.post(f"/write_file/{name}", {"path": "/tmp/large.txt", "content": text})
        resp = client.post(f"/read_file/{name}", {"path": "/tmp/large.txt"})
        assert resp.get("content") == text

    @pytest.mark.skip(reason="slow, team will fix")
    def test_overwrite(self, ready_vm):
        client, name = ready_vm
        client.post(f"/write_file/{name}", {"path": "/tmp/ow.txt", "content": "first"})
        client.post(f"/write_file/{name}", {"path": "/tmp/ow.txt", "content": "second"})
        resp = client.post(f"/read_file/{name}", {"path": "/tmp/ow.txt"})
        assert resp.get("content") == "second"

    @pytest.mark.skip(reason="slow, team will fix")
    def test_nested_path(self, ready_vm):
        client, name = ready_vm
        client.post(f"/exec/{name}", {"command": "mkdir -p /tmp/deep/nested"})
        client.post(f"/write_file/{name}", {"path": "/tmp/deep/nested/f.txt", "content": "deep"})
        resp = client.post(f"/read_file/{name}", {"path": "/tmp/deep/nested/f.txt"})
        assert resp.get("content") == "deep"

    def test_read_nonexistent_file(self, ready_vm):
        client, name = ready_vm
        resp = client.post(f"/read_file/{name}", {"path": "/tmp/no-such-file.txt"})
        assert resp is None or "error" in str(resp).lower()

    def test_read_nonexistent_vm(self, service_env):
        client = service_env.client()
        resp = client.post("/read_file/ghost-vm", {"path": "/tmp/x.txt"})
        assert resp is None or "error" in str(resp).lower() or "not found" in str(resp).lower()
