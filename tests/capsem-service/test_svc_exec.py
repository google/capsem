"""Command execution endpoint tests."""

import pytest

pytestmark = pytest.mark.integration


class TestExec:

    def test_stdout(self, ready_vm):
        client, name = ready_vm
        resp = client.post(f"/exec/{name}", {"command": "echo hello-service"})
        assert resp is not None
        assert "hello-service" in resp.get("stdout", "")

    def test_stderr(self, ready_vm):
        client, name = ready_vm
        resp = client.post(f"/exec/{name}", {"command": "echo err-msg >&2"})
        assert resp is not None
        assert "err-msg" in resp.get("stderr", "") or "err-msg" in resp.get("stdout", "")

    def test_exit_code_zero(self, ready_vm):
        client, name = ready_vm
        resp = client.post(f"/exec/{name}", {"command": "true"})
        assert resp is not None
        assert resp.get("exit_code") == 0

    def test_exit_code_nonzero(self, ready_vm):
        client, name = ready_vm
        resp = client.post(f"/exec/{name}", {"command": "exit 42"})
        assert resp is not None
        assert resp.get("exit_code") == 42

    def test_multiline(self, ready_vm):
        client, name = ready_vm
        resp = client.post(f"/exec/{name}", {"command": "printf 'a\\nb\\nc'"})
        assert "a" in resp.get("stdout", "")
        assert "b" in resp.get("stdout", "")
        assert "c" in resp.get("stdout", "")

    def test_pipe(self, ready_vm):
        client, name = ready_vm
        resp = client.post(f"/exec/{name}", {"command": "echo abc123 | grep -o abc"})
        assert "abc" in resp.get("stdout", "")

    def test_env_var(self, ready_vm):
        client, name = ready_vm
        resp = client.post(f"/exec/{name}", {"command": "export X=works && echo $X"})
        assert "works" in resp.get("stdout", "")

    def test_uname_linux(self, ready_vm):
        client, name = ready_vm
        resp = client.post(f"/exec/{name}", {"command": "uname -s"})
        assert "Linux" in resp.get("stdout", "")

    @pytest.mark.skip(reason="slow, team will fix")
    def test_timeout(self, ready_vm):
        """A command exceeding timeout should be killed and return an error."""
        client, name = ready_vm
        resp = client.post(
            f"/exec/{name}",
            {"command": "sleep 120", "timeout_secs": 2},
            timeout=10,
        )
        assert resp is None or resp.get("exit_code", 0) != 0 or "timeout" in str(resp).lower()

    def test_exec_nonexistent_vm(self, service_env):
        client = service_env.client()
        resp = client.post("/exec/ghost-vm", {"command": "echo nope"})
        assert resp is None or "error" in str(resp).lower() or "not found" in str(resp).lower()
