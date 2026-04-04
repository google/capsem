"""CLI command integration tests: create, list, exec, info, status, delete, run, resume, persist, purge."""

import uuid

import pytest

import subprocess
from pathlib import Path

from helpers.service import wait_exec_ready

PROJECT_ROOT = Path(__file__).parent.parent.parent
CLI_BINARY = PROJECT_ROOT / "target/debug/capsem"


def run_cli(*args, uds_path=None, timeout=60):
    """Run capsem CLI and return (stdout, stderr, returncode)."""
    cmd = [str(CLI_BINARY)]
    if uds_path:
        cmd += ["--uds-path", str(uds_path)]
    cmd += list(args)
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    return result.stdout, result.stderr, result.returncode

pytestmark = pytest.mark.integration


def _provision_vm(uds_path, name, persistent=False):
    """Provision a VM via the service API (non-blocking, for test setup)."""
    from helpers.uds_client import UdsHttpClient
    client = UdsHttpClient(uds_path)
    body = {"name": name, "ram_mb": 2048, "cpus": 2}
    if persistent:
        body["persistent"] = True
    return client.post("/provision", body)


class TestRun:

    def test_run_returns_output(self, uds_path):
        """capsem run executes in a fresh temp VM and returns output."""
        stdout, stderr, rc = run_cli("run", "echo cli-run-works", uds_path=uds_path)
        assert rc == 0, f"run failed: {stderr}"
        assert "cli-run-works" in stdout


class TestList:

    def test_list_empty(self, uds_path):
        # May have VMs from other tests, just verify it doesn't crash
        stdout, stderr, rc = run_cli("list", uds_path=uds_path)
        assert rc == 0, f"list failed: {stderr}"

    def test_list_shows_created_vm(self, uds_path):
        name = f"lsi-{uuid.uuid4().hex[:4]}"
        _provision_vm(uds_path, name)
        try:
            stdout, _, rc = run_cli("list", uds_path=uds_path)
            assert rc == 0
            assert name in stdout
        finally:
            run_cli("delete", name, uds_path=uds_path)


class TestExec:

    def test_exec_echo(self, uds_path):
        name = f"ex1-{uuid.uuid4().hex[:4]}"
        _provision_vm(uds_path, name)
        try:
            from helpers.uds_client import UdsHttpClient
            client = UdsHttpClient(uds_path)
            assert wait_exec_ready(client, name), f"VM {name} never exec-ready"

            stdout, stderr, rc = run_cli("exec", name, "echo cli-works", uds_path=uds_path)
            assert rc == 0, f"exec failed: {stderr}"
            assert "cli-works" in stdout
        finally:
            run_cli("delete", name, uds_path=uds_path)

    def test_exec_nonzero_exit(self, uds_path):
        name = f"ex1-{uuid.uuid4().hex[:4]}"
        _provision_vm(uds_path, name)
        try:
            from helpers.uds_client import UdsHttpClient
            client = UdsHttpClient(uds_path)
            assert wait_exec_ready(client, name)

            _, _, rc = run_cli("exec", name, "exit 42", uds_path=uds_path)
            assert rc == 42
        finally:
            run_cli("delete", name, uds_path=uds_path)


class TestInfo:

    def test_info_json(self, uds_path):
        name = f"ex1-{uuid.uuid4().hex[:4]}"
        _provision_vm(uds_path, name)
        try:
            stdout, stderr, rc = run_cli("info", name, uds_path=uds_path)
            assert rc == 0, f"info failed: {stderr}"
            assert name in stdout
        finally:
            run_cli("delete", name, uds_path=uds_path)


class TestStatus:

    def test_status_shows_running(self, uds_path):
        name = f"st-{uuid.uuid4().hex[:4]}"
        _provision_vm(uds_path, name)
        try:
            stdout, stderr, rc = run_cli("status", name, uds_path=uds_path)
            assert rc == 0, f"status failed: {stderr}"
            assert "Running" in stdout or name in stdout
        finally:
            run_cli("delete", name, uds_path=uds_path)


class TestDelete:

    def test_delete(self, uds_path):
        name = f"ex1-{uuid.uuid4().hex[:4]}"
        _provision_vm(uds_path, name)
        stdout, stderr, rc = run_cli("delete", name, uds_path=uds_path)
        assert rc == 0, f"delete failed: {stderr}"
        # Verify gone from list
        list_out, _, _ = run_cli("list", uds_path=uds_path)
        assert name not in list_out

    def test_delete_nonexistent(self, uds_path):
        _, _, rc = run_cli("delete", "no-such-vm-xyz", uds_path=uds_path)
        assert rc != 0


class TestAliases:

    def test_rm_alias(self, uds_path):
        """capsem rm <id> deletes a VM (alias for delete)."""
        name = f"rmal-{uuid.uuid4().hex[:4]}"
        _provision_vm(uds_path, name)
        stdout, stderr, rc = run_cli("rm", name, uds_path=uds_path)
        assert rc == 0, f"rm alias failed: {stderr}"
        list_out, _, _ = run_cli("list", uds_path=uds_path)
        assert name not in list_out

    def test_ls_alias(self, uds_path):
        """capsem ls returns same data as capsem list."""
        name = f"lsal-{uuid.uuid4().hex[:4]}"
        _provision_vm(uds_path, name)
        try:
            list_out, _, list_rc = run_cli("list", uds_path=uds_path)
            ls_out, _, ls_rc = run_cli("ls", uds_path=uds_path)
            assert list_rc == 0
            assert ls_rc == 0
            assert name in list_out
            assert name in ls_out
        finally:
            run_cli("delete", name, uds_path=uds_path)


class TestStop:

    def test_stop_via_cli(self, uds_path):
        """capsem stop routes to the stop endpoint."""
        name = f"stp-{uuid.uuid4().hex[:4]}"
        _provision_vm(uds_path, name)
        stdout, stderr, rc = run_cli("stop", name, uds_path=uds_path)
        assert rc == 0, f"stop failed: {stderr}"
        assert "stopped" in stdout.lower() or "Stopped" in stdout


class TestPurge:

    def test_purge_via_cli(self, uds_path):
        """capsem purge kills temporary VMs."""
        name = f"prg-{uuid.uuid4().hex[:4]}"
        _provision_vm(uds_path, name)
        stdout, stderr, rc = run_cli("purge", uds_path=uds_path)
        assert rc == 0, f"purge failed: {stderr}"
        assert "Purged" in stdout or "purged" in stdout.lower()

    def test_purge_all_requires_confirmation(self, uds_path):
        """capsem purge --all must prompt and abort on 'n'."""
        name = f"prc-{uuid.uuid4().hex[:4]}"
        _provision_vm(uds_path, name, persistent=True)
        # Pipe "n" to stdin -- should abort without destroying
        cmd = [str(CLI_BINARY), "--uds-path", str(uds_path), "purge", "--all"]
        result = subprocess.run(cmd, input="n\n", capture_output=True, text=True, timeout=30)
        assert result.returncode == 0, f"purge --all with 'n' failed: {result.stderr}"
        assert "Aborted" in result.stdout, (
            f"purge --all should print 'Aborted' when user says no, got: {result.stdout}"
        )
        # VM should still exist
        from helpers.uds_client import UdsHttpClient
        client = UdsHttpClient(uds_path)
        listing = client.get("/list")
        ids = [s["id"] for s in listing["sandboxes"]]
        assert name in ids, f"Persistent VM {name} was destroyed despite user saying 'n'"
        # Cleanup
        client.delete(f"/delete/{name}")

    def test_purge_all_confirmed_destroys(self, uds_path):
        """capsem purge --all with 'y' should destroy persistent VMs."""
        name = f"pry-{uuid.uuid4().hex[:4]}"
        _provision_vm(uds_path, name, persistent=True)
        cmd = [str(CLI_BINARY), "--uds-path", str(uds_path), "purge", "--all"]
        result = subprocess.run(cmd, input="y\n", capture_output=True, text=True, timeout=30)
        assert result.returncode == 0, f"purge --all with 'y' failed: {result.stderr}"
        assert "Purged" in result.stdout
        # VM should be gone
        from helpers.uds_client import UdsHttpClient
        client = UdsHttpClient(uds_path)
        listing = client.get("/list")
        ids = [s["id"] for s in listing["sandboxes"]]
        assert name not in ids, f"Persistent VM {name} survived purge --all with 'y'"


class TestErrors:

    def test_no_service(self):
        """CLI should fail gracefully when service socket doesn't exist."""
        _, stderr, rc = run_cli("list", uds_path="/tmp/nonexistent.sock", timeout=5)
        assert rc != 0
        assert "connect" in stderr.lower() or "error" in stderr.lower()
