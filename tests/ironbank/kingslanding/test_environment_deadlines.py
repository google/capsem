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
from helpers.bounded import bounded
from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.gateway import TcpHttpClient
from helpers.sdk_packages import python_gateway, typescript_gateway
from helpers.service import exec_output_text, vm_name
from helpers.service_gateway import ServiceGateway

from tests.fixtures.oci.registry import publish_catalog, registry
from tests.ironbank.kingslanding.test_environment_startup import (
    IMAGE_CONFIG,
    STARTUP,
    create,
    observe,
    request,
    save,
)
from tests.ironbank.kingslanding.test_image_catalog import entry, use_catalog
from tests.ironbank.kingslanding.test_run import cli, environment, service

__all__ = ["service"]
pytestmark = pytest.mark.integration
ROOT = Path(__file__).resolve().parents[3]
IMAGE = {"Env": ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"],
         "WorkingDir": "/workspace", "Entrypoint": ["/bin/sh", "-c"],
         "Cmd": ['test "$(cat /workspace/long-startup)" = retained; exec redis-server --loglevel warning']}
OLD_GATEWAY_BUDGET = 120
COLD_STARTUP_SECONDS = OLD_GATEWAY_BUDGET + 8


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


@pytest.mark.parametrize("operation", ["start", "resume", "clone"])
def test_gateway_cold_lifecycle_waits_for_the_retained_revision(
    operation, service, tmp_path
):
    assert_cold_lifecycle_waits(
        service, tmp_path, "gateway", operation, COLD_STARTUP_SECONDS
    )


@pytest.mark.parametrize(
    ("client", "operation"),
    [
        ("cli", "resume"),
        ("cli", "clone"),
        ("rust", "start"),
        ("rust", "resume"),
        ("rust", "clone"),
    ],
)
def test_client_cold_lifecycle_waits_for_the_retained_revision(
    client, operation, service, tmp_path
):
    # The Rust SDK previously allowed 110 seconds plus the 120-second gateway allowance.
    startup_seconds = 238 if client == "rust" else COLD_STARTUP_SECONDS
    assert_cold_lifecycle_waits(service, tmp_path, client, operation, startup_seconds)


def run_cold_client(client, operation, service, gateway, source, source_name, timeout):
    target_name = vm_name("cold-clone")
    env = environment(service)
    if client == "cli":
        args = (
            [
                "create",
                "--from",
                source_name,
                "-n",
                target_name,
                "--ram",
                str(DEFAULT_RAM_MB // 1024),
                "--cpu",
                str(DEFAULT_CPUS),
            ]
            if operation == "clone"
            else ["resume", source]
        )
        command = cli(service, *args)
    else:
        command = bounded(
            [
                "cargo",
                "run",
                "--quiet",
                "--locked",
                "--offline",
                "-p",
                "capsem-sdk",
                "--example",
                "gateway_acceptance",
                "--",
                operation,
            ],
            timeout,
        )
        env.update(
            SDK_GATEWAY_URL=gateway.base_url,
            SDK_GATEWAY_TOKEN=gateway.token,
            SDK_VM_ID=source,
            SDK_COLD_SOURCE_NAME=source_name,
            SDK_COLD_TARGET_NAME=target_name,
            SDK_COLD_MEMORY_GIB=str(DEFAULT_RAM_MB // 1024),
            SDK_COLD_CPUS=str(DEFAULT_CPUS),
        )
    result = subprocess.run(
        command,
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
        timeout=timeout + 10,
        check=False,
    )
    assert result.returncode == 0, (result.stdout, result.stderr)
    target = result.stdout.split()[0]
    uuid.UUID(target)
    return target


def assert_cold_lifecycle_waits(service, tmp_path, client, operation, startup_seconds):
    artifacts = {}
    with registry(tmp_path, image_config=IMAGE_CONFIG, artifacts=artifacts) as (
        image,
        certificate,
        _,
    ):
        authority = image.split("/", 1)[0]
        publish_catalog(artifacts, "library/catalog", "stable", entry(image))
        use_catalog(service, f"{authority}/library/catalog:stable", certificate)
        script = STARTUP.removesuffix("sleep 3\n") + (
            f'if [ "$count" -gt 0 ]; then sleep {startup_seconds}; fi\n'
        )
        specification = {
            "name": "cold-startup",
            "image": {"reference": "redis"},
            "startup": {"script": script, "timeout_seconds": startup_seconds + 30},
            "variables": {"BOUND_REVISION": "retained-variable"},
        }
        selected = save(service, specification)
        source_name = vm_name("cold-startup")
        source = create(service, selected, source_name)
        original_generation = observe(service, source, selected, 1)
        request(service, "POST", f"/vms/{source}/stop", {})
        changed = {
            **specification,
            "startup": {"script": "exit 99", "timeout_seconds": 1},
        }
        head = request(
            service,
            "POST",
            f"/environments/{selected['environment_id']}/changes",
            {
                "request_id": str(uuid.uuid4()),
                "base_revision_id": selected["revision_id"],
                "specification": changed,
                "reason": "the shared head must not retarget a cold launch",
            },
        )["result"]
        assert head != selected

        gateway = ServiceGateway(service)
        gateway.start()
        api = TcpHttpClient(gateway.base_url, gateway.token)
        try:
            intent = "clone" if operation == "clone" else "start"
            lookup = source_name if operation == "clone" else source
            readiness = api.get(f"/vms/{lookup}/readiness?intent={intent}")
            assert readiness == {
                "selection": {
                    "id": source,
                    "generation": original_generation,
                    "running": False,
                    "environment": selected,
                },
                "startup_timeout_seconds": startup_seconds + 30,
            }
            if operation == "clone":
                path = "/vms/create"
                payload = {
                    "name": vm_name("cold-clone"),
                    "from": source_name,
                    "ram_mb": DEFAULT_RAM_MB,
                    "cpus": DEFAULT_CPUS,
                }
            else:
                path = f"/vms/{source}/{operation}"
                payload = (
                    {"expected": readiness["selection"]}
                    if operation == "resume"
                    else {}
                )
            started = time.monotonic()
            if client == "gateway":
                code, result = api.call_json(
                    "POST", path, payload, timeout=startup_seconds + 75
                )
                assert code == 200, (code, result)
                target = result["id"]
            else:
                target = run_cold_client(
                    client,
                    operation,
                    service,
                    gateway,
                    source,
                    source_name,
                    startup_seconds + 75,
                )
            assert time.monotonic() - started >= startup_seconds
            assert (target != source) == (operation == "clone")
            try:
                generation = observe(service, target, selected, 2)
                assert generation != original_generation
                if operation != "clone":
                    code, error = api.call_json(
                        "POST", path, {"expected": readiness["selection"]}
                    )
                    assert code == 409, (code, error)
                    assert observe(service, target, selected, 2) == generation
            finally:
                request(service, "DELETE", f"/vms/{target}/delete")
        finally:
            api.close()
            gateway.stop()
