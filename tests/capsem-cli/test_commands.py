"""CLI command integration tests: start, list, exec, info, status, delete."""

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


class TestStart:

    def test_start_with_name(self, uds_path):
        name = f"ex1-{uuid.uuid4().hex[:4]}"
        stdout, stderr, rc = run_cli("start", "--name", name, uds_path=uds_path)
        assert rc == 0, f"start failed: {stderr}"
        assert name in stdout or "ID" in stdout
        run_cli("delete", name, uds_path=uds_path)

    def test_start_without_name(self, uds_path):
        stdout, stderr, rc = run_cli("start", uds_path=uds_path)
        assert rc == 0, f"start failed: {stderr}"
        # Extract the ID from output (e.g. "Sandbox started with ID: vm-xxx")
        for line in stdout.splitlines():
            if "ID:" in line or "id:" in line.lower():
                vm_id = line.split(":")[-1].strip()
                break
        else:
            pytest.fail(f"Could not extract VM ID from output: {stdout}")
        run_cli("delete", vm_id, uds_path=uds_path)


class TestList:

    def test_list_empty(self, uds_path):
        # May have VMs from other tests, just verify it doesn't crash
        stdout, stderr, rc = run_cli("list", uds_path=uds_path)
        assert rc == 0, f"list failed: {stderr}"

    def test_list_shows_created_vm(self, uds_path):
        name = f"lsi-{uuid.uuid4().hex[:4]}"
        run_cli("start", "--name", name, uds_path=uds_path)
        try:
            stdout, _, rc = run_cli("list", uds_path=uds_path)
            assert rc == 0
            assert name in stdout
        finally:
            run_cli("delete", name, uds_path=uds_path)


class TestExec:

    def test_exec_echo(self, uds_path):
        name = f"ex1-{uuid.uuid4().hex[:4]}"
        run_cli("start", "--name", name, uds_path=uds_path)
        try:
            # Wait for VM to be ready (exec via service client)
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
        run_cli("start", "--name", name, uds_path=uds_path)
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
        run_cli("start", "--name", name, uds_path=uds_path)
        try:
            stdout, stderr, rc = run_cli("info", name, uds_path=uds_path)
            assert rc == 0, f"info failed: {stderr}"
            assert name in stdout
        finally:
            run_cli("delete", name, uds_path=uds_path)


class TestStatus:

    def test_status_shows_running(self, uds_path):
        name = f"st-{uuid.uuid4().hex[:4]}"
        run_cli("start", "--name", name, uds_path=uds_path)
        try:
            stdout, stderr, rc = run_cli("status", name, uds_path=uds_path)
            assert rc == 0, f"status failed: {stderr}"
            assert "Running" in stdout or name in stdout
        finally:
            run_cli("delete", name, uds_path=uds_path)


class TestDelete:

    def test_delete(self, uds_path):
        name = f"ex1-{uuid.uuid4().hex[:4]}"
        run_cli("start", "--name", name, uds_path=uds_path)
        stdout, stderr, rc = run_cli("delete", name, uds_path=uds_path)
        assert rc == 0, f"delete failed: {stderr}"
        # Verify gone from list
        list_out, _, _ = run_cli("list", uds_path=uds_path)
        assert name not in list_out

    def test_delete_nonexistent(self, uds_path):
        _, _, rc = run_cli("delete", "no-such-vm-xyz", uds_path=uds_path)
        assert rc != 0


class TestRmFlag:

    def test_start_with_rm_flag(self, uds_path):
        """capsem start --rm provisions successfully."""
        name = f"ex1-{uuid.uuid4().hex[:4]}"
        stdout, stderr, rc = run_cli("start", "--rm", "--name", name, uds_path=uds_path)
        assert rc == 0, f"start --rm failed: {stderr}"
        assert name in stdout or "ID" in stdout
        # Cleanup (--rm only auto-removes on process exit, not immediately)
        run_cli("delete", name, uds_path=uds_path)

    def test_start_rm_then_exec(self, uds_path):
        """VM created with --rm is fully usable for exec."""
        name = f"rmex-{uuid.uuid4().hex[:4]}"
        stdout, stderr, rc = run_cli("start", "--rm", "--name", name, uds_path=uds_path)
        assert rc == 0, f"start --rm failed: {stderr}"
        try:
            from helpers.uds_client import UdsHttpClient
            client = UdsHttpClient(uds_path)
            assert wait_exec_ready(client, name), f"VM {name} never exec-ready"

            stdout, stderr, rc = run_cli("exec", name, "echo rm-works", uds_path=uds_path)
            assert rc == 0, f"exec failed: {stderr}"
            assert "rm-works" in stdout
        finally:
            run_cli("delete", name, uds_path=uds_path)


class TestAliases:

    def test_rm_alias(self, uds_path):
        """capsem rm <id> deletes a VM (alias for delete)."""
        name = f"rmal-{uuid.uuid4().hex[:4]}"
        run_cli("start", "--name", name, uds_path=uds_path)
        stdout, stderr, rc = run_cli("rm", name, uds_path=uds_path)
        assert rc == 0, f"rm alias failed: {stderr}"
        list_out, _, _ = run_cli("list", uds_path=uds_path)
        assert name not in list_out

    def test_ls_alias(self, uds_path):
        """capsem ls returns same data as capsem list."""
        name = f"lsal-{uuid.uuid4().hex[:4]}"
        run_cli("start", "--name", name, uds_path=uds_path)
        try:
            list_out, _, list_rc = run_cli("list", uds_path=uds_path)
            ls_out, _, ls_rc = run_cli("ls", uds_path=uds_path)
            assert list_rc == 0
            assert ls_rc == 0
            assert name in list_out
            assert name in ls_out
        finally:
            run_cli("delete", name, uds_path=uds_path)


class TestErrors:

    def test_no_service(self):
        """CLI should fail gracefully when service socket doesn't exist."""
        _, stderr, rc = run_cli("list", uds_path="/tmp/nonexistent.sock", timeout=5)
        assert rc != 0
        assert "connect" in stderr.lower() or "error" in stderr.lower()
