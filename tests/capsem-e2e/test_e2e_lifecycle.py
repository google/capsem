"""E2E lifecycle tests: real CLI binary, real service, real VMs.

Every test runs the actual capsem binary via subprocess and checks
stdout/stderr. This is what the user does with just shell / just exec.
"""

import uuid

import pytest

pytestmark = pytest.mark.e2e


def _name(prefix="life"):
    return f"{prefix}-{uuid.uuid4().hex[:8]}"


class TestStartExecDelete:
    """The core user flow: start a VM, run a command, check output, delete."""

    def test_start_exec_delete(self, service):
        name = _name()
        service.cli_ok("create", "-n", name)
        assert service.wait_exec_ready(name), f"VM {name} never exec-ready"

        r = service.cli_ok("exec", name, "echo capsem-works")
        assert "capsem-works" in r.stdout

        service.cli_ok("delete", name)

    def test_exec_multiline(self, service):
        name = _name()
        service.cli_ok("create", "-n", name)
        assert service.wait_exec_ready(name)

        r = service.cli_ok("exec", name, "printf 'line1\\nline2\\nline3'")
        assert "line1" in r.stdout
        assert "line2" in r.stdout
        assert "line3" in r.stdout

        service.cli_ok("delete", name)

    def test_exec_exit_code_nonzero(self, service):
        name = _name()
        service.cli_ok("create", "-n", name)
        assert service.wait_exec_ready(name)

        r = service.cli("exec", name, "exit 42")
        # CLI should propagate or report nonzero exit
        assert r.returncode != 0 or "42" in r.stdout or "42" in r.stderr

        service.cli_ok("delete", name)

    def test_exec_stderr(self, service):
        name = _name()
        service.cli_ok("create", "-n", name)
        assert service.wait_exec_ready(name)

        r = service.cli_ok("exec", name, "echo err-msg >&2")
        combined = r.stdout + r.stderr
        assert "err-msg" in combined

        service.cli_ok("delete", name)

    def test_exec_pipe(self, service):
        name = _name()
        service.cli_ok("create", "-n", name)
        assert service.wait_exec_ready(name)

        r = service.cli_ok("exec", name, "echo abc123 | grep -o abc")
        assert "abc" in r.stdout

        service.cli_ok("delete", name)

    def test_exec_env_var(self, service):
        name = _name()
        service.cli_ok("create", "-n", name)
        assert service.wait_exec_ready(name)

        r = service.cli_ok("exec", name, "export X=works && echo $X")
        assert "works" in r.stdout

        service.cli_ok("delete", name)

    def test_exec_uname_linux(self, service):
        """VM must be running Linux."""
        name = _name()
        service.cli_ok("create", "-n", name)
        assert service.wait_exec_ready(name)

        r = service.cli_ok("exec", name, "uname -s")
        assert "Linux" in r.stdout

        service.cli_ok("delete", name)


class TestList:

    def test_list_succeeds(self, service):
        r = service.cli_ok("list")
        # Should not crash, output format varies
        assert r.returncode == 0

    def test_list_shows_created_vm(self, service):
        name = _name("lst")
        service.cli_ok("create", "-n", name)
        try:
            r = service.cli_ok("list")
            assert name in r.stdout
        finally:
            service.cli("delete", name)

    def test_list_omits_deleted_vm(self, service):
        name = _name("del")
        service.cli_ok("create", "-n", name)
        service.cli_ok("delete", name)
        r = service.cli_ok("list")
        assert name not in r.stdout


class TestInfo:

    def test_info_shows_vm(self, service):
        name = _name("inf")
        service.cli_ok("create", "-n", name)
        try:
            r = service.cli_ok("info", name)
            assert name in r.stdout
        finally:
            service.cli("delete", name)

    def test_info_nonexistent(self, service):
        r = service.cli("info", "ghost-vm-404")
        assert r.returncode != 0 or "error" in r.stderr.lower() or "not found" in r.stdout.lower()


class TestDelete:

    def test_delete_nonexistent(self, service):
        r = service.cli("delete", "no-such-vm-xyz")
        assert r.returncode != 0 or "error" in r.stderr.lower() or "not found" in r.stdout.lower()


class TestDoctor:

    def test_doctor_passes(self, service):
        """capsem doctor must pass — it boots a fresh VM and runs diagnostics."""
        r = service.cli_ok("doctor", timeout=120)
        assert r.returncode == 0
