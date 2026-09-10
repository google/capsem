"""Use packaged SDKs through real gateway authentication and routes."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path
from typing import Literal

import pytest
from helpers.gateway import GatewayInstance
from helpers.service import ServiceInstance, materialize_test_profiles
from helpers.workspace_changes import VM_ID, seed_workspace_changes

ROOT = Path(__file__).resolve().parents[2]


@pytest.mark.integration
@pytest.mark.parametrize("language", ["python", "typescript"])
def test_sdk_against_real_gateway_and_stopped_workspace(language: Literal["python", "typescript"]) -> None:
    service = ServiceInstance()
    gateway = GatewayInstance(service.uds_path)
    project = ROOT / "sdk" / language
    try:
        if language == "typescript":
            build = subprocess.run(["node", "tools/build.mjs"], cwd=project,
                                   capture_output=True, text=True, timeout=60, check=False)
            assert build.returncode == 0, build.stdout + build.stderr
        service.profiles_dir = materialize_test_profiles(service.tmp_dir)
        seed_workspace_changes(service.tmp_dir, service.profiles_dir)
        service.start()
        gateway.start()
        result = subprocess.run(
            ["uv", "run", "--project", str(project), "--frozen", "python", "-m", "tests.gateway_acceptance"]
            if language == "python" else ["node", "tools/gateway-acceptance.mjs"],
            cwd=project,
            env={**{key: value for key, value in os.environ.items() if key != "VIRTUAL_ENV"},
                 "SDK_GATEWAY_URL": f"http://127.0.0.1:{gateway.port}",
                 "SDK_GATEWAY_TOKEN": gateway.token, "SDK_VM_ID": VM_ID},
            capture_output=True, text=True, timeout=60, check=False,
        )
        assert result.returncode == 0, result.stdout + result.stderr
        assert "SDK_GATEWAY_ACCEPTANCE_OK" in result.stdout
    finally:
        gateway.stop()
        service.stop()
