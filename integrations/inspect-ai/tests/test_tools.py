"""Inspect sandbox tools binary resolution and baking tests."""

from __future__ import annotations

import gzip
import io
import logging
import tarfile
from pathlib import Path
from types import SimpleNamespace
from typing import Any, cast

import inspect_ai
import inspect_capsem._tools as tools_mod
import inspect_capsem._transfer as xfer_mod
import pytest
from capsem.models import ExecOutput, ExecOutputEncoding
from inspect_capsem._controller import SdkCapsemController
from inspect_capsem._tools import (
    INSPECT_SANDBOX_TOOLS_GUEST_PATH,
    INSPECT_SANDBOX_TOOLS_ONEDIR_PATH,
    bake_sandbox_tools_into_controller,
    resolve_inspect_sandbox_tools_host_binary,
)

from .helpers import LocalFakeCapsemController, Scripted, fail


def found_name() -> str:
    found = resolve_inspect_sandbox_tools_host_binary()
    assert found is not None
    return found.name


def test_resolve_tools_binary_candidates(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("INSPECT_CAPSEM_SANDBOX_TOOLS_PATH", raising=False)
    assert resolve_inspect_sandbox_tools_host_binary(tmp_path / "missing") is None
    real = tmp_path / "tools"
    real.write_bytes(b"x")
    assert resolve_inspect_sandbox_tools_host_binary(real) == real
    monkeypatch.setenv("INSPECT_CAPSEM_SANDBOX_TOOLS_PATH", str(real))
    assert resolve_inspect_sandbox_tools_host_binary() == real
    monkeypatch.setenv("INSPECT_CAPSEM_SANDBOX_TOOLS_PATH", str(tmp_path / "nope"))

    pkg = tmp_path / "pkg"
    binaries = pkg / "binaries"
    binaries.mkdir(parents=True)
    monkeypatch.setattr(inspect_ai, "__file__", str(pkg / "__init__.py"))
    monkeypatch.setattr(tools_mod.platform, "machine", lambda: "x86_64")
    assert resolve_inspect_sandbox_tools_host_binary() is None
    (binaries / "inspect-sandbox-tools-amd64-musl-v9").write_bytes(b"m")
    assert found_name().endswith("musl-v9")
    for name in (
        "inspect-sandbox-tools-amd64-v3",
        "inspect-sandbox-tools-amd64-v12",
        "inspect-sandbox-tools-amd64-v40.tar",
    ):
        (binaries / name).write_bytes(b"b")
    assert found_name() == "inspect-sandbox-tools-amd64-v12"
    (binaries / "inspect-sandbox-tools-amd64-linux").write_bytes(b"e")
    assert found_name() == "inspect-sandbox-tools-amd64-linux"
    monkeypatch.setattr(inspect_ai, "__file__", None)
    assert resolve_inspect_sandbox_tools_host_binary() is None


@pytest.fixture
def gz_tools(tmp_path: Path) -> Path:
    path = tmp_path / "tools.gz"
    path.write_bytes(gzip.compress(b"payload"))
    return path


async def test_bake_via_workspace_and_transfer(
    gz_tools: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("HOME", str(tmp_path))
    xfer_hit = Scripted()
    assert await bake_sandbox_tools_into_controller(xfer_hit, "vm", host_binary_path=gz_tools)
    assert len(xfer_hit.uploads) == 0 and len(xfer_hit.commands) == 1

    xfer = Scripted([("test -x", fail())])
    assert await bake_sandbox_tools_into_controller(xfer, "vm", host_binary_path=gz_tools)
    assert xfer.uploads[INSPECT_SANDBOX_TOOLS_GUEST_PATH] == gz_tools.read_bytes()
    assert len(xfer.uploads) == 1 and len(xfer.commands) == 2

    xfer_fail = Scripted([("test -x", fail()), ("chmod 700", fail(stderr="ro fs"))])
    assert not await bake_sandbox_tools_into_controller(xfer_fail, "vm", host_binary_path=gz_tools)

    pws = tmp_path / ".capsem" / "run" / "persistent" / "vm" / "workspace"
    pws.mkdir(parents=True)
    xfer = Scripted([("test -x", fail())])
    assert await bake_sandbox_tools_into_controller(xfer, "vm", host_binary_path=gz_tools)
    assert not any("/root/.capsem_stage_tools" in c for c in xfer.commands)
    assert xfer.uploads[INSPECT_SANDBOX_TOOLS_GUEST_PATH] == gz_tools.read_bytes()


async def test_bake_without_workspace_uses_files_api_parts(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The ~14 MiB tools binary must reach an ephemeral VM in parts under the chunk cap."""
    monkeypatch.setattr(xfer_mod, "_XFER_PART_BYTES", 4)
    writes: list[bytes] = []
    commands: list[str] = []
    created: list[dict[str, Any]] = []

    class Files:
        async def write(self, path: str, data: bytes) -> None:
            del path
            writes.append(data)

    class Session:
        id = "vm-e"
        files = Files()

        async def exec(
            self, command: str, *, timeout_secs: int | None = None, target: Any = None
        ) -> Any:
            del timeout_secs, target
            commands.append(command)
            return SimpleNamespace(
                exit_code=1 if command.startswith("test -x") else 0,
                stdout=ExecOutput(data="", encoding=ExecOutputEncoding.UTF8),
                stderr=ExecOutput(data="", encoding=ExecOutputEncoding.UTF8),
                truncated=False,
            )

        async def delete(self) -> None:
            return None

    class Hv:
        async def create(self, **kwargs: Any) -> Any:
            created.append(kwargs)
            return Session()

        async def close(self) -> None:
            return None

    tools = tmp_path / "tools"
    tools.write_bytes(b"0123456789")

    ctrl = SdkCapsemController(hypervisor=cast(Any, Hv()))
    try:
        vid = await ctrl.start_vm(cpu_count=1, ram_gb=1)
        assert created[0].get("name") is None
        assert created[0].get("labels", {}).get("managed-by") == "inspect-capsem"
        assert await bake_sandbox_tools_into_controller(ctrl, vid, host_binary_path=tools)
    finally:
        await ctrl.close()

    assert writes == [b"0123", b"4567", b"89"]
    assert any(
        c.startswith("mkdir -p") and f"/* > {INSPECT_SANDBOX_TOOLS_GUEST_PATH}" in c
        for c in commands
    )


async def test_bake_failures(gz_tools: Path, tmp_path: Path) -> None:
    assert not await bake_sandbox_tools_into_controller(
        Scripted(), "vm", host_binary_path=tmp_path / "no"
    )
    assert await bake_sandbox_tools_into_controller(Scripted(), "vm", host_binary_path=gz_tools)
    ctrl = Scripted([("test -x", fail()), ("chmod 700", fail())])
    assert not await bake_sandbox_tools_into_controller(ctrl, "vm", host_binary_path=gz_tools)
    plain = tmp_path / "plain"
    plain.write_bytes(b"not gzip")
    ctrl = Scripted([("test -x", fail())])
    assert await bake_sandbox_tools_into_controller(ctrl, "vm", host_binary_path=plain)
    assert not any("tar -xzf" in c for c in ctrl.commands)


async def test_resolve_and_bake_sandbox_tools_v29_and_warning(
    tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    """Resolves versioned inspect-sandbox-tools binary, extracts tar.gz, and warns on missing."""
    resolved = resolve_inspect_sandbox_tools_host_binary()
    assert resolved is not None
    assert resolved.is_file()
    assert "inspect-sandbox-tools-" in resolved.name

    controller = LocalFakeCapsemController(tmp_path, skip_bake=False)
    vid = await controller.start_vm(cpu_count=2, ram_gb=4)

    with caplog.at_level(logging.WARNING):
        ok_missing = await bake_sandbox_tools_into_controller(
            cast(Any, controller), vid, host_binary_path=tmp_path / "does-not-exist"
        )
    assert ok_missing is False
    assert any("Could not locate host inspect-sandbox-tools" in r.message for r in caplog.records)

    buf = io.BytesIO()
    with (
        gzip.GzipFile(fileobj=buf, mode="wb") as gz,
        tarfile.open(fileobj=gz, mode="w") as tf,
    ):
        payload = b"#!/bin/sh\necho sandbox-tools-ok\n"
        info = tarfile.TarInfo(name="inspect-sandbox-tools")
        info.size = len(payload)
        info.mode = 0o755
        tf.addfile(info, io.BytesIO(payload))
    fake_tar_gz = tmp_path / "inspect-sandbox-tools-amd64-v29"
    fake_tar_gz.write_bytes(buf.getvalue())

    ok_baked = await bake_sandbox_tools_into_controller(
        cast(Any, controller), vid, host_binary_path=fake_tar_gz
    )
    assert ok_baked is True
    vm_root = controller.vms[vid]
    staged = vm_root / INSPECT_SANDBOX_TOOLS_GUEST_PATH.lstrip("/")
    extracted = vm_root / INSPECT_SANDBOX_TOOLS_ONEDIR_PATH.lstrip("/") / "inspect-sandbox-tools"
    assert staged.is_file()
    assert extracted.is_file()
