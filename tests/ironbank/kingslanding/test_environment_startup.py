"""Saved image/startup acceptance through public routes and a TLS OCI catalog.

Only the configured catalog CA supplies registry trust. The fixture entrypoint
records what startup prepared, proving ordering before a real Redis workload.
"""

import concurrent.futures
import copy
import secrets
import time
import uuid

import pytest
from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import exec_output_text, vm_name

from tests.fixtures.oci.registry import publish_catalog, registry
from tests.ironbank.kingslanding.test_image_catalog import entry, use_catalog
from tests.ironbank.kingslanding.test_run import service

__all__ = ["service"]

pytestmark = pytest.mark.integration

STARTUP = """set -eu
count=0
if [ -f /workspace/start-count ]; then count=$(cat /workspace/start-count); fi
printf '%s' "$((count + 1))" > /workspace/start-count
printf '%s' "$BOUND_REVISION" > /workspace/prepared-value
printf 'approved startup output\\n'
sleep 3
"""

IMAGE_CONFIG = {
    "Env": ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"],
    "WorkingDir": "/workspace",
    "Entrypoint": ["/bin/sh", "-c"],
    "Cmd": [
        'set -eu; test "$(cat /workspace/prepared-value)" = "$BOUND_REVISION"; '
        'cp /workspace/start-count /workspace/entrypoint-count; '
        'printf started >> /workspace/entrypoint-runs; exec redis-server'
    ],
}


def request(service, method, path, body=None, *, expected=200):
    code, value = service.client().call_json(method, path, body, timeout=90)
    assert code == expected, (method, path, code, value)
    return value


def save(service, specification):
    saved = request(service, "POST", "/environments/create", {
        "request_id": str(uuid.uuid4()), "specification": specification,
        "reason": "hermetic startup acceptance",
    })
    assert saved["application"] == "applied", saved
    return saved["result"]


def create(service, reference, name):
    return request(service, "POST", "/vms/create", {
        "name": name, "environment": reference,
        "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
    })["id"]


def observe(service, vm, reference, count):
    info = request(service, "GET", f"/vms/{vm}/info")["environment"]
    assert info["boot"] == reference, info
    output = request(service, "POST", f"/vms/{vm}/exec", {"command": (
        'printf "%s:%s:%s:%s" "$BOUND_REVISION" "$(cat /workspace/start-count)" '
        '"$(cat /workspace/prepared-value)" "$(cat /workspace/entrypoint-count)"'
    )})
    assert output["exit_code"] == 0, output
    assert exec_output_text(output) == f"retained-variable:{count}:retained-variable:{count}"
    assert request(service, "GET", f"/vms/{vm}/container")["state"] == "running"
    redis = request(service, "POST", f"/vms/{vm}/exec", {"command": "redis-cli PING"})
    assert redis["exit_code"] == 0 and exec_output_text(redis).strip() == "PONG", redis
    return info["owner_generation"]


def test_saved_startup_precedes_entrypoint_on_create_cold_resume_and_fork(service, tmp_path):
    artifacts = {}
    with registry(tmp_path, image_config=IMAGE_CONFIG, artifacts=artifacts) as (image, certificate, requests):
        authority = image.split("/", 1)[0]
        publish_catalog(artifacts, "library/catalog", "stable", entry(image))
        use_catalog(service, f"{authority}/library/catalog:stable", certificate)
        specification = {"name": "startup-acceptance", "image": {"reference": "redis"},
                         "startup": {"script": STARTUP, "timeout_seconds": 10},
                         "variables": {"BOUND_REVISION": "retained-variable"}}
        reference = save(service, specification)
        name = vm_name("startup-order")
        saw_preparing = False
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
            future = executor.submit(create, service, reference, name)
            deadline = time.monotonic() + 90
            while time.monotonic() < deadline and not future.done():
                rows = request(service, "GET", "/vms/list")["sandboxes"]
                for row in rows:
                    if row.get("name") != name:
                        continue
                    code, status = service.client().call_json("GET", f'/vms/{row["id"]}/container', timeout=5)
                    if code == 200 and status["state"] == "preparing":
                        assert set(status) == {"state", "image", "digest", "resolved"}, status
                        saw_preparing = True
                time.sleep(0.02)
            vm = future.result(timeout=5)
        assert saw_preparing, "startup must be observable before create becomes ready"
        first = observe(service, vm, reference, 1)
        assert any(path.endswith(f'/manifests/{image.rsplit("@", 1)[1]}') for path in requests)

        changed = copy.deepcopy(specification)
        changed["startup"]["script"] = "exit 99"
        edited = request(service, "POST", f'/environments/{reference["environment_id"]}/changes', {
            "request_id": str(uuid.uuid4()), "base_revision_id": reference["revision_id"],
            "specification": changed, "reason": "shared edits must not retarget existing owners",
        })
        assert edited["result"]["revision_id"] != reference["revision_id"]
        request(service, "POST", f"/vms/{vm}/stop", {})
        request(service, "POST", f"/vms/{vm}/resume", {})
        resumed = observe(service, vm, reference, 2)
        assert resumed != first
        fork = request(service, "POST", f"/vms/{vm}/fork", {"name": vm_name("startup-fork")})["id"]
        request(service, "POST", f"/vms/{fork}/resume", {})
        assert observe(service, fork, reference, 3) not in {first, resumed}
        observe(service, vm, reference, 2)
        managed_id, token = str(uuid.uuid4()), secrets.token_hex(32)
        managed = request(service, "POST", "/vms/create", {
            "environment": reference,
            "managed": {"request_id": managed_id, "ownership_token": token},
            "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
        })
        observe(service, managed["id"], reference, 1)
        closed = request(service, "POST", f"/vms/managed/{managed_id}/close", {
            "ownership_token": token, "outcome": "completed",
        })
        assert closed["state"] == "closed", closed


@pytest.mark.parametrize(("action", "failure"), [
    ("exit", "startup exit (17)"), ("timeout", "startup timed_out"),
    ("remove-shell", "startup entrypoint_exec"),
])
def test_failed_startup_blocks_entrypoint_and_retains_workspace(service, tmp_path, action, failure):
    artifacts = {}
    script = """set -eu
action=$(cat /workspace/startup-action 2>/dev/null || :)
case "$action" in
exit) printf preserved > /workspace/failure-evidence; printf '\\033[31m'; head -c 70000 /dev/zero | tr '\\000' x; exit 17;;
timeout) printf timer-evidence; sleep 30;;
remove-shell) rm /bin/sh;;
esac
""" + STARTUP.replace("sleep 3\n", "")
    with registry(tmp_path, image_config=IMAGE_CONFIG, artifacts=artifacts) as (image, certificate, _):
        authority = image.split("/", 1)[0]
        publish_catalog(artifacts, "library/catalog", "stable", entry(image))
        use_catalog(service, f"{authority}/library/catalog:stable", certificate)
        specification = {"name": "startup-failure", "image": {"reference": "redis"},
                         "startup": {"script": script, "timeout_seconds": 2},
                         "variables": {"BOUND_REVISION": "retained-variable"}}
        reference = save(service, specification)
        vm = create(service, reference, vm_name("startup-failure"))
        observe(service, vm, reference, 1)
        client = service.client()
        code, _, data = client.call("POST", f"/vms/{vm}/files/content?path=/workspace/startup-action",
                                   body=action.encode(), headers={"Content-Type": "application/octet-stream"})
        assert code == 200, (code, data)
        request(service, "POST", f"/vms/{vm}/stop", {})
        error = request(service, "POST", f"/vms/{vm}/resume", {}, expected=500)
        status = request(service, "GET", f"/vms/{vm}/container")
        assert status["state"] == "failed", status
        assert failure in status["error"] and failure in error["error"], (status, error)
        assert len(status["error"].encode()) < 64 * 1024
        assert "\x1b" not in status["error"]
        code, _, data = client.call("GET", f"/vms/{vm}/files/content?path=/workspace/entrypoint-runs")
        assert code == 200 and data == b"started", (code, data)
        if action == "exit":
            assert status["error"].endswith("[startup output truncated]"), status
            code, _, data = client.call("GET", f"/vms/{vm}/files/content?path=/workspace/failure-evidence")
            assert code == 200 and data == b"preserved", (code, data)
        if action == "remove-shell":
            request(service, "POST", f"/vms/{vm}/stop", {})
            error = request(service, "POST", f"/vms/{vm}/resume", {}, expected=500)
            assert "startup shell_unavailable" in error["error"], error
            status = request(service, "GET", f"/vms/{vm}/container")
            assert status["state"] == "failed" and "startup shell_unavailable" in status["error"], status
