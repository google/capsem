"""Per-sandbox history endpoints: /history/{id}, /processes, /counts, /transcript."""

import base64
import uuid

import pytest

pytestmark = pytest.mark.integration


def _run(client, name, command):
    """Exec a command in the VM; return the response dict."""
    return client.post(f"/exec/{name}", {"command": command, "timeout_secs": 30})


class TestHistoryList:

    def test_history_returns_executed_commands(self, ready_vm):
        """/history/{id} returns commands that were executed in the VM."""
        client, name = ready_vm

        marker = f"history-probe-{uuid.uuid4().hex[:6]}"
        _run(client, name, f"echo {marker}")

        resp = client.get(f"/history/{name}")
        assert resp is not None
        commands = resp.get("commands")
        assert isinstance(commands, list), f"commands not a list: {resp}"
        assert resp.get("total", 0) >= 1, f"total=0 after exec: {resp}"

        # The marker may appear either in command text or stdout_preview depending on the layer.
        haystack = " ".join(
            (entry.get("command") or "") + " " + (entry.get("stdout_preview") or "")
            for entry in commands
        )
        assert marker in haystack, (
            f"marker {marker!r} not surfaced in history: {commands[:3]}"
        )

    def test_history_pagination(self, ready_vm):
        """limit=N bounds the commands array; has_more tracks remainder."""
        client, name = ready_vm

        # Run a few more commands so pagination is meaningful.
        for i in range(3):
            _run(client, name, f"echo pg-{i}-{uuid.uuid4().hex[:4]}")

        resp = client.get(f"/history/{name}?limit=1&offset=0")
        assert resp is not None
        assert len(resp["commands"]) <= 1, f"limit=1 returned {len(resp['commands'])}"
        if resp["total"] > 1:
            assert resp["has_more"] is True, f"has_more false despite total>{resp}"

    def test_history_nonexistent_vm(self, client):
        resp = client.get(f"/history/ghost-{uuid.uuid4().hex[:6]}")
        assert resp is None or "error" in resp or "not found" in str(resp).lower()


class TestHistoryProcesses:

    def test_processes_shape(self, ready_vm):
        """/history/{id}/processes returns a list of ProcessEntry objects."""
        client, name = ready_vm
        _run(client, name, "true")

        resp = client.get(f"/history/{name}/processes")
        assert resp is not None
        processes = resp.get("processes")
        assert isinstance(processes, list), f"processes not a list: {resp}"
        # Each entry should have the documented fields. Don't require non-empty
        # because audit_count may be 0 in a fresh VM depending on telemetry timing.
        for entry in processes:
            assert "exe" in entry, f"missing exe: {entry}"
            assert "command_count" in entry
            assert "first_seen" in entry
            assert "last_seen" in entry


class TestHistoryCounts:

    def test_counts_nonnegative(self, ready_vm):
        """/history/{id}/counts returns non-negative integer counts."""
        client, name = ready_vm
        _run(client, name, "true")

        resp = client.get(f"/history/{name}/counts")
        assert resp is not None
        assert "exec_count" in resp and "audit_count" in resp, f"missing counts: {resp}"
        assert isinstance(resp["exec_count"], int) and resp["exec_count"] >= 0
        assert isinstance(resp["audit_count"], int) and resp["audit_count"] >= 0
        # After at least one exec, exec_count should have moved.
        assert resp["exec_count"] >= 1, f"exec_count did not increment: {resp}"


class TestHistoryTranscript:

    def test_transcript_base64_decodable(self, ready_vm):
        """/history/{id}/transcript returns base64-encoded content and accurate byte count."""
        client, name = ready_vm

        resp = client.get(f"/history/{name}/transcript")
        assert resp is not None
        content = resp.get("content", "")
        bytes_len = resp.get("bytes", -1)
        assert isinstance(content, str)
        assert isinstance(bytes_len, int) and bytes_len >= 0
        # Empty log is allowed (pty.log missing returns bytes=0 per handler),
        # but when content is non-empty the byte count must match decoded length.
        if content:
            decoded = base64.b64decode(content)
            assert len(decoded) == bytes_len, (
                f"bytes={bytes_len} does not match decoded len={len(decoded)}"
            )
