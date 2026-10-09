"""OCI workload container staging, healthcheck, and working_dir probing tests."""

from __future__ import annotations

from pathlib import Path
from typing import Any, cast

import inspect_capsem.containers.runtime as rt_mod
import pytest
from inspect_capsem import CapsemSandboxConfig
from inspect_capsem.containers.runtime import (
    prepare_oci_workload_container,
    resolve_container_working_dir,
)

from ..helpers import Scripted, fail, ok


async def test_prepare_oci_workload_container_and_working_dir_probe(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    ctrl = Scripted([("pwd", ok("/app\n"))])
    spec_default = CapsemSandboxConfig(image="alpine:3.19").to_container_spec()
    await prepare_oci_workload_container(cast(Any, ctrl), "vm-1", spec_default)
    assert ctrl.commands == []
    assert await resolve_container_working_dir(cast(Any, ctrl), "vm-1") == "/app"
    assert (
        await resolve_container_working_dir(cast(Any, Scripted([("pwd", fail(1, ""))])), "vm-1")
        == "/"
    )

    spec_explicit = CapsemSandboxConfig(
        image="alpine:3.19", working_dir="/custom/work"
    ).to_container_spec()
    await prepare_oci_workload_container(cast(Any, ctrl), "vm-1", spec_explicit)
    assert ctrl.commands == [
        "pwd",
        "mkdir -p /custom/work && (chmod a+rx /custom/work 2>/dev/null || true)",
    ]

    monkeypatch.setenv("CAPSEM_INSPECT_ALLOWED_HOST_PATHS", str(tmp_path))
    host_dir, host_file = tmp_path / "data", tmp_path / "single.sh"
    host_dir.mkdir()
    (host_dir / "hello.txt").write_text("hi\n")
    host_file.write_text("#!/bin/sh\necho single\n")
    host_file.chmod(0o755)

    ctrl_vols = Scripted([("", ok())])
    cfg_vols = CapsemSandboxConfig(
        image="alpine:3.19",
        volumes=(f"{host_dir}:/mnt/data:ro", f"{host_file}:/etc/single.sh:ro"),
        allowed_host_paths=(str(tmp_path),),
    )
    await prepare_oci_workload_container(cast(Any, ctrl_vols), "vm-1", cfg_vols.to_container_spec())
    assert ctrl_vols.uploads["/etc/single.sh"] == b"#!/bin/sh\necho single\n"
    assert any("chmod 755 /etc/single.sh" in c for c in ctrl_vols.commands)
    assert any(p.startswith("/tmp/.capsem_bind_") for p in ctrl_vols.uploads)
    assert any("tar -xzf /tmp/.capsem_bind_0.tar.gz -C /mnt/data" in c for c in ctrl_vols.commands)

    spec_single = CapsemSandboxConfig(
        image="alpine:3.19",
        volumes=(f"{host_file}:/etc/single.sh:ro",),
        allowed_host_paths=(str(tmp_path),),
    ).to_container_spec()
    with pytest.raises(RuntimeError, match="Failed creating parent directory"):
        await prepare_oci_workload_container(
            cast(Any, Scripted([("mkdir -p", fail(1, stderr="ro"))])), "vm-1", spec_single
        )

    monkeypatch.setattr(rt_mod, "_MAX_BIND_MOUNT_BYTES", 2)
    for vol in (f"{host_file}:/etc/single.sh:ro", f"{host_dir}:/mnt/data:ro"):
        spec = CapsemSandboxConfig(
            image="alpine:3.19", volumes=(vol,), allowed_host_paths=(str(tmp_path),)
        ).to_container_spec()
        with pytest.raises(ValueError, match="exceeds maximum size"):
            await prepare_oci_workload_container(cast(Any, ctrl_vols), "vm-1", spec)


async def test_oci_healthcheck_polling() -> None:
    ctrl_ok = Scripted([("check_ok", ok())])
    spec_ok = CapsemSandboxConfig(
        image="ubuntu:24.04",
        healthcheck={"test": ["CMD-SHELL", "check_ok"], "retries": "2", "interval": "10ms"},
    ).to_container_spec()
    await prepare_oci_workload_container(ctrl_ok, "vm-1", spec_ok)
    assert any("sh -c check_ok" in c for c in ctrl_ok.commands)

    ctrl_fail = Scripted([("check_hc", fail(1, stderr="hc failed"))])
    spec_fail = CapsemSandboxConfig(
        image="ubuntu:24.04",
        healthcheck={"test": ["CMD", "check_hc"], "interval": 0.01},
    ).to_container_spec()
    with pytest.raises(RuntimeError, match=r"failed healthcheck.*after 3 attempts"):
        await prepare_oci_workload_container(ctrl_fail, "vm-1", spec_fail)
