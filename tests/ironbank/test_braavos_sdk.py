"""Braavos: installed Python/TS packages and source-built Rust use the real gateway."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path
from typing import Literal

import pytest
from helpers.bounded import bounded
from helpers.constants import ASSETS_DIR
from helpers.gateway import GatewayInstance
from helpers.sdk_packages import python_gateway, typescript_gateway
from helpers.service import ServiceInstance
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
        seed_stopped_workspace(service.tmp_dir, ASSETS_DIR)
        service.start()
        gateway.start()
        environment = {**{key: value for key, value in os.environ.items() if key != "VIRTUAL_ENV"},
                       "SDK_GATEWAY_URL": f"http://127.0.0.1:{gateway.port}",
                       "SDK_GATEWAY_TOKEN": gateway.token, "SDK_VM_ID": VM_ID}
        if language == "python":
            output = python_gateway(ROOT, environment)
        elif language == "typescript":
            output = typescript_gateway(ROOT, environment)
        else:
            result = subprocess.run(
                bounded(["cargo", "run", "--frozen", "--quiet", "-p", "capsem-sdk", "--example",
                         "gateway_acceptance"], 60),
                cwd=project, env=environment, capture_output=True, text=True, check=False,
            )
            assert result.returncode == 0, result.stdout + result.stderr
            output = result.stdout
        assert "BRAAVOS_SDK_ACCEPTANCE_OK" in output
        if language != "rust":
            assert "SDK_IMAGE_PACKAGE_ACCEPTANCE_OK" in output, (
                "gateway acceptance must first prove clean installed-package payload and origins"
            )
    finally:
        gateway.stop()
        service.stop()
