"""A normal pinned workload mounts its prepared EROFS without raw extraction."""

import json
import shlex
import time

import pytest
from helpers.image_session import image_session
from helpers.service import ServiceInstance, exec_output_text

from tests.fixtures.oci.pinned_image import debug_image, reference_image
from tests.fixtures.oci.published_root import ready

pytestmark = pytest.mark.integration


def assert_prepared_root(client, vm_id, expected):
    probe = """
import json
from pathlib import Path
mounts = []
for entry in Path("/proc").glob("[0-9]*/cmdline"):
    try:
        argv = entry.read_bytes().split(bytes([0]))
        if b"/root/.capsem-image/launch.py" in argv and b"--detached" in argv:
            mounts.append((entry.parent / "mounts").read_text())
    except FileNotFoundError:
        pass
print(json.dumps({
    "options": json.loads(Path("/root/.capsem-image/options.json").read_text()),
    "mounts": mounts,
    "raw_roots": list(map(str, Path("/var/lib/capsem/roots").glob("*/ready"))),
}))
"""
    result = client.post(
        f"/vms/{vm_id}/exec",
        {
            "target": "vm",
            "command": "python3 -c " + shlex.quote(probe),
            "timeout_secs": 10,
        },
        timeout=20,
    )
    assert result["exit_code"] == 0, result
    actual = json.loads(exec_output_text(result))
    assert actual["options"]["rootfs"] == expected
    assert actual["raw_roots"] == []
    assert len(actual["mounts"]) == 1, actual["mounts"]
    assert any(
        line.split()[1:3] == ["/var/tmp/capsem-container/bundle/rootfs", "erofs"]
        for line in actual["mounts"][0].splitlines()
    ), actual["mounts"]
    workload = client.post(
        f"/vms/{vm_id}/exec",
        {"command": "printf PREPARED_ROOT_READY", "timeout_secs": 10},
        timeout=20,
    )
    assert workload["exit_code"] == 0, workload
    assert exec_output_text(workload) == "PREPARED_ROOT_READY"


@pytest.mark.parametrize("image", [debug_image, reference_image], ids=lambda image: image.section)
def test_prepared_root_is_used_on_create_and_cached_resume(tmp_path, image):
    manifest, _ = ready(image.ready())
    (layer,) = manifest["layers"]
    expected = {key: layer[key] for key in ("digest", "size")}
    expected["integrity"] = manifest["annotations"]["org.capsem.rootfs.blake3"]
    service = ServiceInstance()
    measurements = {"image": image.settings().name}
    try:
        service.start()
        started = time.monotonic()
        with image_session(
            service, tmp_path, "prepared-root-proof", image=image, ram_mb=12288, cpus=4
        ) as vm_id:
            client = service.client()
            measurements["cold_create_s"] = time.monotonic() - started
            assert_prepared_root(client, vm_id, expected)
            for index in range(2):
                stopped = client.post(f"/vms/{vm_id}/stop", {}, timeout=30)
                assert "error" not in stopped, stopped
                started = time.monotonic()
                resumed = client.post(f"/vms/{vm_id}/resume", {}, timeout=110)
                assert "error" not in resumed, resumed
                measurements[f"cached_resume_{index}_s"] = time.monotonic() - started
                assert_prepared_root(client, vm_id, expected)
            reference = client.get(f"/vms/{vm_id}/container")["image"]
            assert "error" not in client.post(f"/vms/{vm_id}/stop", {}, timeout=30)
            for index in range(2):
                started = time.monotonic()
                created = client.post(
                    "/vms/create",
                    {
                        "name": f"prepared-cached-{index}",
                        "ram_mb": 12288,
                        "cpus": 4,
                        "container": {
                            "image": reference,
                            "registry": {"ca_pem": (tmp_path / "registry.pem").read_text()},
                        },
                    },
                    timeout=110,
                )
                assert "error" not in created, created
                cached_id = created["id"]
                try:
                    first = client.post(
                        f"/vms/{cached_id}/exec",
                        {"command": "printf PREPARED_ROOT_READY", "timeout_secs": 10},
                        timeout=20,
                    )
                    assert first["exit_code"] == 0, first
                    assert exec_output_text(first) == "PREPARED_ROOT_READY"
                    measurements[f"cached_create_to_exec_{index}_s"] = time.monotonic() - started
                    assert_prepared_root(client, cached_id, expected)
                finally:
                    client.delete(f"/vms/{cached_id}/delete", timeout=30)
    finally:
        service.stop()
        print("PREPARED_ROOT_TIMINGS=" + json.dumps(measurements))
