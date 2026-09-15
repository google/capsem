"""Reuse Ironbank's hermetic model upstream and inspect it through the SDK."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

import pytest
from helpers.gateway import GatewayInstance
from ironbank.model_client_scripts import openai_responses_api_script

ROOT = Path(__file__).resolve().parents[2]


@pytest.mark.integration
def test_sdk_observes_real_model_and_tool_events(model_client_env, tmp_path: Path) -> None:
    script = tmp_path / "model-proof.py"
    script.write_text(openai_responses_api_script("https://api.openai.com"))
    gateway = GatewayInstance(model_client_env.service.uds_path)
    try:
        gateway.start()
        result = subprocess.run(
            ["uv", "run", "--frozen", "python", "-m", "tests.model_acceptance"],
            cwd=ROOT / "sdk/python", env={
                **{key: value for key, value in os.environ.items() if key != "VIRTUAL_ENV"},
                "SDK_GATEWAY_URL": gateway.base_url, "SDK_GATEWAY_TOKEN": gateway.token,
                "SDK_VM_ID": model_client_env.session_id, "SDK_MODEL_SCRIPT": str(script),
            }, capture_output=True, text=True, timeout=150, check=False,
        )
        assert result.returncode == 0, result.stdout + result.stderr
        assert "SDK_MODEL_ACCEPTANCE_OK" in result.stdout
        print(result.stdout.strip())
    finally:
        gateway.stop()
