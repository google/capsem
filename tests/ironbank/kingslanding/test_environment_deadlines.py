"""Native saved startup exceeds the former public wait without losing ownership."""
from __future__ import annotations

import copy
import os
import subprocess
import time
import uuid
from pathlib import Path

import psutil
import pytest
from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.sdk_packages import python_gateway, typescript_gateway
from helpers.service import exec_output_text, vm_name

from tests.fixtures.oci.registry import publish_catalog, registry
from tests.ironbank.kingslanding.test_environment_startup import request, save
from tests.ironbank.kingslanding.test_image_catalog import entry, use_catalog
from tests.ironbank.kingslanding.test_run import cli, environment, service

__all__ = ["service"]
pytestmark = pytest.mark.integration
ROOT = Path(__file__).resolve().parents[3]
IMAGE = {"Env": ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"],
         "WorkingDir": "/workspace", "Entrypoint": ["/bin/sh", "-c"],
         "Cmd": ['test "$(cat /workspace/long-startup)" = retained; exec redis-server --loglevel warning']}


def retained(service, image, certificate, artifacts):
    authority = image.split("/", 1)[0]
    publish_catalog(artifacts, "library/catalog", "stable", entry(image))
    use_catalog(service, f"{authority}/library/catalog:stable", certificate)
    settings = service.home_dir / "settings.toml"
    settings.write_text(settings.read_text() + "\n[managed_sessions]\nlease_seconds=30\nrenew_after_seconds=5\n")
    request(service, "POST", "/corp/reload", {})
    specification = {"name": "long-startup", "image": {"reference": "redis"},
                     "startup": {"script": "sleep 235\nprintf retained > /workspace/long-startup\n", "timeout_seconds": 300}}
    selected = save(service, specification)
    changed = copy.deepcopy(specification)
    changed["startup"] = {"script": "exit 99", "timeout_seconds": 1}
    edited = request(service, "POST", f'/environments/{selected["environment_id"]}/changes', {
        "request_id": str(uuid.uuid4()), "base_revision_id": selected["revision_id"],
        "specification": changed, "reason": "exact retained timing must ignore the new shared head",
    })
    assert edited["result"]["revision_id"] != selected["revision_id"]
    return selected


def assert_no_native_owner(service):
    assert request(service, "GET", "/vms/list")["sandboxes"] == []
    assert service.proc is not None
    assert not [child.pid for child in psutil.Process(service.proc.pid).children()
                if child.is_running() and child.status() != psutil.STATUS_ZOMBIE and child.name() == "capsem-process"]


def test_cli_saved_creation_waits_for_retained_long_startup(service, tmp_path):
    artifacts = {}
    with registry(tmp_path, image_config=IMAGE, artifacts=artifacts) as (image, certificate, _):
        selected = retained(service, image, certificate, artifacts)
        started = time.monotonic()
        ran = subprocess.run([*cli(service, "create"), "-n", vm_name("long-startup"),
                              "--ram", str(DEFAULT_RAM_MB // 1024), "--cpu", str(DEFAULT_CPUS),
                              "--environment", f'{selected["environment_id"]}@{selected["revision_id"]}'],
                             env=environment(service), capture_output=True, timeout=360, check=False)
        assert ran.returncode == 0, (ran.stdout, ran.stderr)
        assert time.monotonic() - started >= 235
        vm = ran.stdout.decode().split()[0]
        uuid.UUID(vm)
        try:
            ready = request(service, "POST", f"/vms/{vm}/exec", {
                "command": 'test "$(cat /workspace/long-startup)" = retained; redis-cli PING'})
            assert ready["exit_code"] == 0 and exec_output_text(ready) == "PONG\n", ready
            assert request(service, "GET", f"/vms/{vm}/info")["environment"]["boot"] == selected
            assert request(service, "GET", f"/vms/{vm}/container")["state"] == "running"
        finally:
            request(service, "DELETE", f"/vms/{vm}/delete")
        assert_no_native_owner(service)


@pytest.mark.parametrize("language", ["python", "typescript"])
def test_installed_sdk_renews_long_preparation_and_cancels_original_owner(language, service, tmp_path):
    artifacts = {}
    with registry(tmp_path, image_config=IMAGE, artifacts=artifacts) as (image, certificate, _):
        selected = retained(service, image, certificate, artifacts)
        port = int((service.tmp_dir / "gateway.port").read_text())
        settings = {**{key: value for key, value in os.environ.items() if key != "VIRTUAL_ENV"},
                    "SDK_GATEWAY_URL": f"http://127.0.0.1:{port}",
                    "SDK_GATEWAY_TOKEN": (service.tmp_dir / "gateway.token").read_text().strip(),
                    "SDK_ENVIRONMENT_ID": selected["environment_id"], "SDK_REVISION_ID": selected["revision_id"]}
        run = python_gateway if language == "python" else typescript_gateway
        output = run(ROOT, settings,
                     probe="startup_acceptance.py" if language == "python" else "startup-acceptance.mjs",
                     success_marker="SDK_STARTUP_DEADLINE_OK", timeout_seconds=360)
        assert "SDK_IMAGE_PACKAGE_ACCEPTANCE_OK" in output and "SDK_STARTUP_DEADLINE_OK" in output
        assert_no_native_owner(service)
