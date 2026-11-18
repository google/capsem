"""Attached saved workloads preserve approval, output lanes and exit status."""

import concurrent.futures
import re
import secrets
import time
import uuid

import msgpack
import pytest
from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import vm_name
from websockets.sync.client import connect
from websockets.typing import Subprotocol

from tests.fixtures.oci.registry import publish_catalog, registry
from tests.ironbank.kingslanding.test_environment_startup import request, save
from tests.ironbank.kingslanding.test_image_catalog import entry, use_catalog
from tests.ironbank.kingslanding.test_run import service

__all__ = ["service"]
pytestmark = pytest.mark.integration

IMAGE = {
    "Env": ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"],
    "WorkingDir": "/workspace", "Entrypoint": ["/bin/sh", "-c"],
    "Cmd": ['printf "workload-%s:%s" "$BOUND_REVISION" "$(cat /workspace/count)"; '
            'printf workload-error >&2; exit 7'],
}
STARTUP = """set -eu
count=0
if [ -f /workspace/count ]; then count=$(cat /workspace/count); fi
printf '%s' "$((count + 1))" > /workspace/count
printf 'startup-output-only\\n'
sleep 2
"""


def attach(service, vm, *, expected_error=None):
    port = int((service.tmp_dir / "gateway.port").read_text())
    token = (service.tmp_dir / "gateway.token").read_text().strip()
    output = {"stdout": b"", "stderr": b""}
    started = False
    with connect(f"ws://127.0.0.1:{port}/vms/{vm}/stream",
                 subprotocols=[Subprotocol("capsem.stream.v3")],
                 additional_headers={"Authorization": f"Bearer {token}"}, open_timeout=10) as socket:
        socket.send(msgpack.packb({"control": {"type": "start", "kind": "container"}}, use_bin_type=True))
        socket.send(msgpack.packb({"control": {"type": "close_stdin"}}, use_bin_type=True))
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            frame = socket.recv(timeout=max(0.01, deadline - time.monotonic()))
            assert isinstance(frame, bytes), frame
            message = msgpack.unpackb(frame)
            assert len(message) == 1, message
            if "status" in message:
                status = message["status"]
                if status["type"] == "started":
                    assert status == {"type": "started"} and not started, status
                    started = True
                elif status["type"] == "exit":
                    assert set(status) == {"type", "code", "truncated"}, status
                    assert started and not status["truncated"], status
                    return status["code"], output
                elif status["type"] == "error" and expected_error is not None:
                    assert set(status) == {"type", "message"}, status
                    assert not started and expected_error in status["message"], status
                    return None, output
                else:
                    raise AssertionError(status)
            else:
                (lane,) = message
                assert started and lane in output and isinstance(message[lane], bytes), message
                output[lane] += message[lane]
    raise AssertionError("attached workload did not publish its exit")


def staged(service, reference, **options):
    vm = request(service, "POST", "/vms/create", {
        "environment": reference, "attach": True,
        "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS, **options,
    })["id"]
    assert request(service, "GET", f"/vms/{vm}/container")["state"] == "staged"
    assert request(service, "GET", f"/vms/{vm}/info")["environment"]["boot"] == reference
    return vm


def test_attached_saved_startup_keeps_stdout_separate_and_clone_and_managed_transport_work(service, tmp_path):
    artifacts = {}
    with registry(tmp_path, image_config=IMAGE, artifacts=artifacts) as (image, certificate, _):
        authority = image.split("/", 1)[0]
        publish_catalog(artifacts, "library/catalog", "stable", entry(image))
        use_catalog(service, f"{authority}/library/catalog:stable", certificate)
        reference = save(service, {"name": "attached-startup", "image": {"reference": "redis"},
                                  "startup": {"script": STARTUP, "timeout_seconds": 10},
                                  "variables": {"BOUND_REVISION": "retained-variable"}})
        name = vm_name("attached-startup")
        vm = staged(service, reference, name=name)
        saw_preparing = False
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
            future = executor.submit(attach, service, vm)
            while not future.done():
                status = request(service, "GET", f"/vms/{vm}/container")
                if status["state"] == "preparing":
                    saw_preparing = True
                time.sleep(0.02)
            code, output = future.result()
        assert saw_preparing
        assert code == 7 and output["stdout"] == b"workload-retained-variable:1", (code, output)
        assert output["stderr"].endswith(b"workload-error"), output
        request(service, "POST", f"/vms/{vm}/stop", {})
        clone = staged(service, reference, name=vm_name("attached-clone"), **{"from": name})
        code, output = attach(service, clone)
        assert code == 7 and output["stdout"] == b"workload-retained-variable:2", (code, output)
        managed_id, token = str(uuid.uuid4()), secrets.token_hex(32)
        managed = staged(service, reference, managed={"request_id": managed_id, "ownership_token": token})
        code, output = attach(service, managed)
        assert code == 7 and output["stdout"] == b"workload-retained-variable:1", (code, output)
        closed = request(service, "POST", f"/vms/managed/{managed_id}/close", {
            "ownership_token": token, "outcome": "completed",
        })
        assert closed["state"] == "closed", closed


@pytest.mark.parametrize(("script", "code", "failure"), [
    ("printf startup-failure; exit 17", 17, "startup exit (17)"),
    ("printf startup-timeout; sleep 30", 124, "startup timed_out"),
])
def test_attached_preparation_failure_never_runs_the_entrypoint(service, tmp_path, script, code, failure):
    artifacts = {}
    with registry(tmp_path, image_config=IMAGE, artifacts=artifacts) as (image, certificate, _):
        authority = image.split("/", 1)[0]
        publish_catalog(artifacts, "library/catalog", "stable", entry(image))
        use_catalog(service, f"{authority}/library/catalog:stable", certificate)
        reference = save(service, {"name": "attached-failure", "image": {"reference": "redis"},
                                  "startup": {"script": script, "timeout_seconds": 1}})
        vm = staged(service, reference, name=vm_name("attached-failure"))
        actual, output = attach(service, vm)
        assert actual == code and output["stdout"] == b"", (actual, output)
        status = request(service, "GET", f"/vms/{vm}/container")
        assert status["state"] == "failed" and failure in status["error"], status
        assert len(status["error"].encode()) < 64 * 1024


def test_attached_transport_without_an_environment_image_fails_before_vm_allocation(service):
    reference = save(service, {"name": "plain-policy", "variables": {"BOUND_REVISION": "plain"}})
    before = request(service, "GET", "/vms/list")
    error = request(service, "POST", "/vms/create", {"environment": reference, "attach": True}, expected=400)
    assert "requires an image" in error["error"], error
    assert request(service, "GET", "/vms/list") == before


def test_attached_dispatch_rechecks_image_admission_after_staging(service, tmp_path):
    artifacts = {}
    with registry(tmp_path, image_config=IMAGE, artifacts=artifacts) as (image, certificate, _):
        authority = image.split("/", 1)[0]
        publish_catalog(artifacts, "library/catalog", "stable", entry(image))
        use_catalog(service, f"{authority}/library/catalog:stable", certificate)
        reference = save(service, {"name": "attached-revocation", "image": {"reference": "redis"},
                                  "startup": {"script": STARTUP, "timeout_seconds": 10}})
        vm = staged(service, reference, name=vm_name("attached-revocation"))
        settings = service.home_dir / "settings.toml"
        before = settings.read_text()
        settings.write_text(re.sub(r"\[images\]\n(?:[^\[\n][^\n]*\n)*", "", before)
                            + "\n[images]\ncatalog = false\n")
        try:
            code, output = attach(service, vm, expected_error="image")
            assert code is None and output == {"stdout": b"", "stderr": b""}, (code, output)
            assert request(service, "GET", f"/vms/{vm}/container")["state"] == "failed"
        finally:
            settings.write_text(before)
