"""Braavos: use all packaged SDKs through real gateway authentication and routes."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path
from typing import Literal

import pytest
from helpers.gateway import GatewayInstance
from helpers.service import ServiceInstance, materialize_test_profiles
from helpers.stopped_workspace import VM_ID, seed_stopped_workspace

ROOT = Path(__file__).resolve().parents[2]


@pytest.mark.integration
@pytest.mark.parametrize("language", ["python", "typescript", "rust"])
def test_braavos_sdk_against_real_gateway_and_stopped_workspace(
    language: Literal["python", "typescript", "rust"],
) -> None:
    service = ServiceInstance()
    gateway = GatewayInstance(service.uds_path)
    project = ROOT / "sdk" / language
    try:
        service.profiles_dir = materialize_test_profiles(service.tmp_dir)
        seed_stopped_workspace(service.tmp_dir, service.profiles_dir)
        service.start()
        gateway.start()
        # The gate prepares each SDK before the suites (`sdk.python.sync`,
        # `functional.sdk.rust.example`, `functional.sdk.typescript.bundle`);
        # these run with no network, so they only consume what it made.
        commands = {
            "python": ["uv", "run", "--project", str(project), "--frozen", "--no-sync",
                       "python", "-m", "tests.gateway_acceptance"],
            "typescript": ["node", "tools/gateway-acceptance.mjs"],
            "rust": ["cargo", "run", "--frozen", "--quiet", "-p", "capsem-sdk", "--example", "gateway_acceptance"],
        }
        result = subprocess.run(
            commands[language],
            cwd=project,
            env={**{key: value for key, value in os.environ.items() if key != "VIRTUAL_ENV"},
                 "SDK_GATEWAY_URL": f"http://127.0.0.1:{gateway.port}",
                 "SDK_GATEWAY_TOKEN": gateway.token, "SDK_VM_ID": VM_ID},
            capture_output=True, text=True, timeout=60, check=False,
        )
        assert result.returncode == 0, result.stdout + result.stderr
        assert "BRAAVOS_SDK_ACCEPTANCE_OK" in result.stdout
    finally:
        gateway.stop()
        service.stop()
