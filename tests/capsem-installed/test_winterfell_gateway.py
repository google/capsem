"""Installed-cohort persistence proof through authenticated gateway HTTP."""

from __future__ import annotations

import os
import uuid
from contextlib import suppress
from pathlib import Path

import pytest
from helpers.constants import (
    DEFAULT_CPUS,
    DEFAULT_RAM_MB,
    EXEC_READY_TIMEOUT,
)
from helpers.gateway import TcpHttpClient
from helpers.image_session import WORKSPACE, image_session, wait_running, workload_exec
from helpers.service import (
    ServiceInstance,
    installed_exec_output_text,
    resolve_winterfell_artifact_roots,
    vm_record,
    wait_exec_ready,
)

pytestmark = [
    pytest.mark.integration,
    pytest.mark.skipif(
        "CAPSEM_WINTERFELL_BIN_DIR" not in os.environ,
        reason="runs only against an explicit installed Winterfell cohort",
    ),
]


def test_installed_gateway_persists_exec_state() -> None:
    roots = resolve_winterfell_artifact_roots()
    assert roots.installed
    service = ServiceInstance(assets_dir=roots.assets_dir, sign_binaries=False)
    vm_id: str | None = None
    try:
        service.start()
        port = (service.tmp_dir / "gateway.port").read_text().strip()
        token = (service.tmp_dir / "gateway.token").read_text().strip()
        gateway = TcpHttpClient(f"http://127.0.0.1:{port}", token)
        denied = TcpHttpClient(f"http://127.0.0.1:{port}", "incorrect-token")
        assert denied.call_json("GET", "/vms/list")[0] == 401

        name = f"winterfell-{uuid.uuid4().hex[:8]}"
        status, created = gateway.call_json(
            "POST",
            "/vms/create",
            {
                "name": name,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
            },
            timeout=120,
        )
        assert status == 200 and isinstance(created, dict), created
        vm_id = created["id"]
        assert wait_exec_ready(
            service.client(),
            vm_id,
            timeout=EXEC_READY_TIMEOUT,
            read=installed_exec_output_text,
        ), vm_record(service.client(), vm_id)

        command = "printf 'the north remembers' > /root/stark_words.txt"
        assert (
            gateway.call_json("POST", f"/vms/{vm_id}/exec", {"command": command})[0]
            == 200
        )
        assert gateway.call_json("POST", f"/vms/{vm_id}/stop")[0] == 200
        assert gateway.call_json("POST", f"/vms/{vm_id}/resume", timeout=120)[0] == 200
        assert wait_exec_ready(
            service.client(),
            vm_id,
            timeout=EXEC_READY_TIMEOUT,
            read=installed_exec_output_text,
        ), vm_record(service.client(), vm_id)
        status, result = gateway.call_json(
            "POST", f"/vms/{vm_id}/exec", {"command": "cat /root/stark_words.txt"}
        )
        assert (
            status == 200
            and installed_exec_output_text(result) == "the north remembers"
        )
    finally:
        if vm_id is not None:
            with suppress(Exception):
                service.client().delete(f"/vms/{vm_id}/delete", timeout=60)
        service.stop()


#: The reference image's layout, staged into the installed proof's inputs by
#: whoever runs it (run-installed-winterfell.py --image-layout). An installed
#: run without it fails: the release proof must boot the product's own shape.
IMAGE_LAYOUT_ENV = "CAPSEM_WINTERFELL_IMAGE_LAYOUT"


def test_installed_image_session_lives_through_stop_resume_and_fork(tmp_path) -> None:
    """The installed package runs a user's image session, through the gateway:
    the reference image (the official `dev`) admitted as one exact image, its
    workload writing the workspace as the image's user, relaunched by the
    VM's boot after a stop, and carried into a fork."""
    layout = os.environ.get(IMAGE_LAYOUT_ENV)
    assert layout, f"installed Winterfell needs {IMAGE_LAYOUT_ENV}: the reference image's layout"
    roots = resolve_winterfell_artifact_roots()
    assert roots.installed
    service = ServiceInstance(assets_dir=roots.assets_dir, sign_binaries=False)
    try:
        service.start()
        port = (service.tmp_dir / "gateway.port").read_text().strip()
        token = (service.tmp_dir / "gateway.token").read_text().strip()
        gateway = TcpHttpClient(f"http://127.0.0.1:{port}", token)
        name = f"winterfell-image-{uuid.uuid4().hex[:8]}"
        with image_session(
            service, tmp_path / "registry", name, layout=Path(layout), client=gateway
        ) as vm_id:
            wrote = workload_exec(
                gateway, vm_id, f"id -u; pwd; echo 'the north remembers' > {WORKSPACE}/stark.txt"
            )
            assert wrote.get("exit_code") == 0, wrote
            assert wrote["stdout_text"].splitlines() == ["1000", WORKSPACE], wrote

            # A stop and a resume: the VM's boot relaunches the staged image.
            assert gateway.call_json("POST", f"/vms/{vm_id}/stop")[0] == 200
            assert gateway.call_json("POST", f"/vms/{vm_id}/resume", timeout=120)[0] == 200
            wait_running(gateway, vm_id, timeout=300)
            kept = workload_exec(gateway, vm_id, f"cat {WORKSPACE}/stark.txt")
            assert kept["stdout_text"] == "the north remembers\n", kept

            # A fork is an image session of the same digest, with the files.
            digest = gateway.get(f"/vms/{vm_id}/container")["digest"]
            fork_id = gateway.post(f"/vms/{vm_id}/fork", {"name": f"{name}-fork"})["id"]
            try:
                gateway.post(f"/vms/{fork_id}/resume", {}, timeout=120)
                wait_running(gateway, fork_id, timeout=300)
                assert gateway.get(f"/vms/{fork_id}/container")["digest"] == digest
                forked = workload_exec(gateway, fork_id, f"cat {WORKSPACE}/stark.txt")
                assert forked["stdout_text"] == "the north remembers\n", forked
            finally:
                with suppress(Exception):
                    service.client().delete(f"/vms/{fork_id}/delete", timeout=60)
    finally:
        service.stop()
