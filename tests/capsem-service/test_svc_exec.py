"""Command execution endpoint tests."""

import pytest
from helpers.service import exec_output_text

pytestmark = pytest.mark.integration


class TestExec:

    def test_stdout(self, ready_vm):
        client, name = ready_vm
        resp = client.post(f"/vms/{name}/exec", {"command": "echo hello-service"})
        assert resp is not None
        assert "hello-service" in exec_output_text(resp)

    def test_stderr(self, ready_vm):
        client, name = ready_vm
        resp = client.post(f"/vms/{name}/exec", {"command": "echo err-msg >&2"})
        assert resp is not None
        assert "err-msg" in exec_output_text(resp, "stderr") or "err-msg" in exec_output_text(resp)

    def test_exit_code_zero(self, ready_vm):
        client, name = ready_vm
        resp = client.post(f"/vms/{name}/exec", {"command": "true"})
        assert resp is not None
        assert resp.get("exit_code") == 0

    def test_exit_code_nonzero(self, ready_vm):
        client, name = ready_vm
        resp = client.post(f"/vms/{name}/exec", {"command": "exit 42"})
        assert resp is not None
        assert resp.get("exit_code") == 42

    def test_multiline(self, ready_vm):
        client, name = ready_vm
        resp = client.post(f"/vms/{name}/exec", {"command": "printf 'a\\nb\\nc'"})
        assert "a" in exec_output_text(resp)
        assert "b" in exec_output_text(resp)
        assert "c" in exec_output_text(resp)

    def test_pipe(self, ready_vm):
        client, name = ready_vm
        resp = client.post(f"/vms/{name}/exec", {"command": "echo abc123 | grep -o abc"})
        assert "abc" in exec_output_text(resp)

    def test_env_var(self, ready_vm):
        client, name = ready_vm
        resp = client.post(f"/vms/{name}/exec", {"command": "export X=works && echo $X"})
        assert "works" in exec_output_text(resp)

    def test_uname_linux(self, ready_vm):
        client, name = ready_vm
        resp = client.post(f"/vms/{name}/exec", {"command": "uname -s"})
        assert "Linux" in exec_output_text(resp)

    def test_timeout(self, ready_vm):
        """A command exceeding timeout should be killed and return an error."""
        client, name = ready_vm
        resp = client.post(
            f"/vms/{name}/exec",
            {"command": "sleep 120", "timeout_secs": 2},
            timeout=10,
        )
        message = str(resp).lower()
        assert (
            resp is None
            or resp.get("exit_code", 0) != 0
            or "timeout" in message
            or "timed out" in message
        )

    def test_exec_nonexistent_vm(self, service_env):
        client = service_env.client()
        resp = client.post("/vms/ghost-vm/exec", {"command": "echo nope"})
        assert resp is None or "error" in str(resp).lower() or "not found" in str(resp).lower()
