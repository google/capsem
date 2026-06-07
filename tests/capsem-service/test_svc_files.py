"""Files API: list workspace, download, upload."""

import uuid

import pytest

pytestmark = pytest.mark.integration


class TestFilesList:

    def test_list_workspace_root(self, ready_vm):
        """GET /files/{id} returns an entries array for the workspace root."""
        client, name = ready_vm
        resp = client.get(f"/files/{name}")
        assert resp is not None
        assert isinstance(resp.get("entries"), list), f"entries not a list: {resp}"

    def test_list_nonexistent_vm(self, client):
        resp = client.get(f"/files/ghost-{uuid.uuid4().hex[:6]}")
        assert resp is None or "error" in resp or "not found" in str(resp).lower()


class TestFilesDownload:

    def test_download_nonexistent_file(self, ready_vm):
        client, name = ready_vm
        status, _body = client.get_bytes(
            f"/files/{name}/content?path=nonexistent-{uuid.uuid4().hex[:6]}.txt"
        )
        assert status == 404, f"expected 404 for missing file, got {status}"


class TestFilesUploadDownload:

    def test_upload_download_roundtrip(self, ready_vm):
        """POST /files/{id}/content writes bytes; GET reads the same bytes back."""
        client, name = ready_vm

        payload = f"upload-roundtrip-{uuid.uuid4().hex}\n".encode() + b"\x00\x01\x02binary-ok"
        filename = f"rt-{uuid.uuid4().hex[:8]}.bin"

        resp = client.post_bytes(f"/files/{name}/content?path={filename}", payload)
        assert resp is not None
        assert resp.get("success") is True, f"upload failed: {resp}"
        assert resp.get("size") == len(payload), (
            f"size {resp.get('size')} != payload {len(payload)}"
        )

        status, body = client.get_bytes(f"/files/{name}/content?path={filename}")
        assert status == 200, f"download status {status}, expected 200"
        assert body == payload, (
            f"roundtrip mismatch: uploaded {len(payload)} bytes, got {len(body)} bytes back"
        )

    def test_upload_overwrites_existing(self, ready_vm):
        """A second upload to the same path replaces the prior content atomically."""
        client, name = ready_vm

        filename = f"overwrite-{uuid.uuid4().hex[:8]}.txt"
        first = b"first-version"
        second = b"second-version-which-is-longer"

        assert client.post_bytes(
            f"/files/{name}/content?path={filename}", first
        ).get("success") is True
        assert client.post_bytes(
            f"/files/{name}/content?path={filename}", second
        ).get("success") is True

        status, body = client.get_bytes(f"/files/{name}/content?path={filename}")
        assert status == 200
        assert body == second, f"expected overwrite, got {body!r}"
