"""Installed-cohort persistence proof through authenticated gateway HTTP."""

from __future__ import annotations

import os
import uuid
from contextlib import suppress

import pytest
from helpers.constants import (
    CODE_PROFILE_ID,
    DEFAULT_CPUS,
    DEFAULT_RAM_MB,
    EXEC_READY_TIMEOUT,
)
from helpers.gateway import TcpHttpClient
from helpers.service import (
    ServiceInstance,
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
    service.profiles_dir = roots.profiles_dir
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
                "profile_id": CODE_PROFILE_ID,
                "ram_mb": DEFAULT_RAM_MB,
                "cpus": DEFAULT_CPUS,
            },
            timeout=120,
        )
        assert status == 200 and isinstance(created, dict), created
        vm_id = created["id"]
        assert wait_exec_ready(service.client(), vm_id, timeout=EXEC_READY_TIMEOUT), (
            vm_record(service.client(), vm_id)
        )

        command = "printf 'the north remembers' > /root/stark_words.txt"
        assert (
            gateway.call_json("POST", f"/vms/{vm_id}/exec", {"command": command})[0]
            == 200
        )
        assert gateway.call_json("POST", f"/vms/{vm_id}/stop")[0] == 200
        assert gateway.call_json("POST", f"/vms/{vm_id}/resume", timeout=120)[0] == 200
        assert wait_exec_ready(service.client(), vm_id, timeout=EXEC_READY_TIMEOUT), (
            vm_record(service.client(), vm_id)
        )
        status, result = gateway.call_json(
            "POST", f"/vms/{vm_id}/exec", {"command": "cat /root/stark_words.txt"}
        )
        assert status == 200 and result["stdout"] == "the north remembers"
    finally:
        if vm_id is not None:
            with suppress(Exception):
                service.client().delete(f"/vms/{vm_id}/delete", timeout=60)
        service.stop()
