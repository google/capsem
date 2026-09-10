"""SDK acceptance owns disposable VMs and communicates through the TCP gateway."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

import pytest
from helpers.gateway import GatewayInstance
from helpers.service import ServiceInstance

ROOT = Path(__file__).resolve().parents[2]


@pytest.mark.integration
def test_python_sdk_live_vm_lifecycle_and_binary_files() -> None:
    service = ServiceInstance()
    gateway = GatewayInstance(service.uds_path)
    try:
        service.start()
        gateway.start()
        result = subprocess.run(
            ["uv", "run", "--frozen", "python", "-m", "tests.live_acceptance"],
            cwd=ROOT / "sdk/python", env={
                **{key: value for key, value in os.environ.items() if key != "VIRTUAL_ENV"},
                "SDK_GATEWAY_URL": gateway.base_url, "SDK_GATEWAY_TOKEN": gateway.token,
            }, capture_output=True, text=True, timeout=240, check=False,
        )
        assert result.returncode == 0, result.stdout + result.stderr
        assert "SDK_LIVE_ACCEPTANCE_OK" in result.stdout
        print(result.stdout.strip())
    finally:
        gateway.stop()
        service.stop()
