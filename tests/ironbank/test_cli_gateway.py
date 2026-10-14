"""CLI consumers use the real authenticated gateway, including installed cohorts."""

from __future__ import annotations

import os
import subprocess
import time
from pathlib import Path

import pytest
from helpers.bounded import bounded
from helpers.constants import BIN_DIR
from helpers.gateway import TcpHttpClient
from helpers.service import ServiceInstance

pytestmark = pytest.mark.integration


def gateway(service: ServiceInstance) -> TcpHttpClient:
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        try:
            port = int((service.tmp_dir / "gateway.port").read_text())
            token = (service.tmp_dir / "gateway.token").read_text().strip()
            client = TcpHttpClient(f"http://127.0.0.1:{port}", token)
            if client.get_raw("/vms/list", timeout=2) == 200:
                return client
        except (OSError, ValueError):
            pass
        time.sleep(0.05)
    raise AssertionError("service-owned gateway did not become ready")


def command(
    service: ServiceInstance, *arguments: str, token: str | None = None,
) -> subprocess.CompletedProcess[bytes]:
    environment = {
        key: value for key, value in os.environ.items()
        if key not in {"CAPSEM_GATEWAY_URL", "CAPSEM_GATEWAY_TOKEN"}
    }
    environment.update({
        "HOME": str(service.home_dir), "CAPSEM_HOME": str(service.home_dir),
        "CAPSEM_RUN_DIR": str(service.tmp_dir),
    })
    if token is not None:
        environment["CAPSEM_GATEWAY_TOKEN"] = token
    return subprocess.run(
        bounded([str(BIN_DIR / "capsem"), "--uds-path", str(service.uds_path), *arguments], 120),
        env=environment, capture_output=True, timeout=130, check=False,
    )


def success(output: subprocess.CompletedProcess[bytes]) -> bytes:
    assert output.returncode == 0, output.stderr.decode(errors="replace")
    return output.stdout


def test_cli_gateway_authentication_never_falls_back_to_live_service() -> None:
    service = ServiceInstance()
    try:
        service.start()
        client = gateway(service)
        assert success(command(service, "list", "--quiet")) == b""
        for arguments in (("list", "--quiet"), ("create", "--name", "must-not-create")):
            denied = command(service, *arguments, token="invalid-cli-test-token")
            assert denied.returncode != 0
            assert b"gateway HTTP 401" in denied.stderr, denied.stderr
            assert b"invalid-cli-test-token" not in denied.stderr + denied.stdout
            assert client.get("/vms/list")["sandboxes"] == []
        token_path = service.tmp_dir / "gateway.token"
        token_bytes = token_path.read_bytes()
        token_path.unlink()
        try:
            missing = command(service, "list", "--quiet")
            assert missing.returncode != 0
            assert b"gateway token unavailable" in missing.stderr, missing.stderr
            # The same service remains reachable: an empty success would be a fallback.
            assert service.client().get("/vms/list")["sandboxes"] == []
        finally:
            token_path.touch(mode=0o600)
            token_path.write_bytes(token_bytes)
    finally:
        service.stop()


def test_cli_gateway_vm_lifecycle_exit_codes_and_binary_files(tmp_path: Path) -> None:
    service = ServiceInstance()
    try:
        service.start()
        client = gateway(service)
        created = success(command(service, "create", "--name", "cli-gateway-proof"))
        session_id = created.decode().split()[0]
        assert session_id in success(command(service, "list", "--quiet")).decode().splitlines()
        executed = command(service, "exec", session_id, "printf CLI_READY; printf CLI_ERROR >&2; exit 23")
        assert executed.returncode == 23, executed.stderr
        assert executed.stdout == b"CLI_READY" and executed.stderr == b"CLI_ERROR"
        original = tmp_path / "original.bin"
        restored = tmp_path / "restored.bin"
        original.write_bytes(bytes(range(256)) * 17)
        guest = f"{session_id}:gateway binary %25.bin"
        success(command(service, "cp", str(original), guest))
        success(command(service, "cp", guest, str(restored)))
        assert restored.read_bytes() == original.read_bytes()
        success(command(service, "fork", session_id, "cli-gateway-fork"))
        success(command(service, "delete", session_id))
        assert client.get("/vms/list")["sandboxes"] == []
    finally:
        service.stop()
