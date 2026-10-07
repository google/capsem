"""Installed SDK consumers own disposable VMs through the real TCP gateway."""

from __future__ import annotations

import os
from pathlib import Path

import pytest
from helpers.gateway import GatewayInstance
from helpers.sdk_packages import python_gateway, typescript_gateway
from helpers.service import ServiceInstance

ROOT = Path(__file__).resolve().parents[2]


@pytest.mark.integration
@pytest.mark.parametrize("language", ["python", "typescript"])
def test_sdk_live_vm_lifecycle_and_binary_files(language: str) -> None:
    service = ServiceInstance()
    gateway = GatewayInstance(service.uds_path)
    try:
        service.start()
        gateway.start()
        environment = {
            **{key: value for key, value in os.environ.items() if key != "VIRTUAL_ENV"},
            "SDK_GATEWAY_URL": gateway.base_url, "SDK_GATEWAY_TOKEN": gateway.token,
        }
        run = python_gateway if language == "python" else typescript_gateway
        output = run(
            ROOT, environment,
            probe="live_acceptance.py" if language == "python" else "live-acceptance.mjs",
            success_marker="SDK_LIVE_ACCEPTANCE_OK", timeout_seconds=240,
        )
        assert "SDK_IMAGE_PACKAGE_ACCEPTANCE_OK" in output
        assert "SDK_LIVE_ACCEPTANCE_OK" in output
        print(output.strip())
    finally:
        gateway.stop()
        service.stop()
