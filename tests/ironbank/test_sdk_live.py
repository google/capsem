"""Installed SDK consumers own disposable VMs through the real TCP gateway."""

from __future__ import annotations

import os
from pathlib import Path

import pytest
from helpers.gateway import GatewayInstance
from helpers.sdk_packages import python_gateway, typescript_gateway
from helpers.service import ServiceInstance

from tests.fixtures.oci.registry import grant_exact, registry

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


@pytest.mark.integration
@pytest.mark.parametrize("language", ["python", "typescript"])
def test_installed_sdk_oci_create_and_execution_targets(language: str, tmp_path: Path) -> None:
    service = ServiceInstance()
    gateway = GatewayInstance(service.uds_path)
    try:
        with registry(tmp_path) as (reference, certificate, requests):
            grant_exact(service.home_dir, reference)
            service.start()
            gateway.start()
            environment = {
                **{key: value for key, value in os.environ.items() if key != "VIRTUAL_ENV"},
                "SDK_GATEWAY_URL": gateway.base_url, "SDK_GATEWAY_TOKEN": gateway.token,
                "SDK_IMAGE": reference, "SDK_REGISTRY_CA": str(certificate),
            }
            run = python_gateway if language == "python" else typescript_gateway
            output = run(
                ROOT, environment,
                probe="oci_acceptance.py" if language == "python" else "oci-acceptance.mjs",
                success_marker="SDK_OCI_ACCEPTANCE_OK", timeout_seconds=240,
            )
            assert "SDK_IMAGE_PACKAGE_ACCEPTANCE_OK" in output
            assert "SDK_OCI_ACCEPTANCE_OK" in output
            digest = reference.split("@", 1)[1]
            assert requests.count(f"/v2/library/redis/manifests/{digest}") == 1, requests
            assert any("/blobs/" in path for path in requests), requests
            assert service.client().get("/vms/list")["sandboxes"] == []
            print(output.strip())
    finally:
        gateway.stop()
        service.stop()
