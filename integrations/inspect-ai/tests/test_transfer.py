"""SDK controller staged file transfer, path validation, and Files API part chunking tests."""

from __future__ import annotations

import base64 as b64
import os
import subprocess as sp
from pathlib import Path
from types import SimpleNamespace
from typing import Any, cast

import capsem
import inspect_capsem._transfer as xfer_mod
import pytest
from capsem import HttpError
from capsem.models import ExecOutput, ExecOutputEncoding
from inspect_capsem import CapsemSandboxEnvironment
from inspect_capsem._controller import CommandResult, SdkCapsemController

from .helpers import Scripted, _host_timeout_skips, _run_inspect_self_check, exec_response, fail, ok


def _server_check_rel_path(path: str) -> None:
    if ".." in path or any(ch in path for ch in (":", "@", "+")):
        raise HttpError(400, '{"code":"invalid_request","error":"invalid path"}')


def test_rel_to_stage_dir_path_validation() -> None:
    for invalid in (
        "/workspace/../etc/passwd",
        "/etc/passwd",
        "/workspace",
        "/workspace/a:b.txt",
        "/workspace/a@b.txt",
        "/workspace/a+b.txt",
        "/workspace/with space.txt",
    ):
        assert xfer_mod._rel_to_stage_dir(invalid, "/workspace") is None
    assert xfer_mod._rel_to_stage_dir("/workspace/sub/ok_1.txt", "/workspace") == "sub/ok_1.txt"
    assert xfer_mod._rel_to_stage_dir("/root/.bashrc", "/root") == ".bashrc"
    assert xfer_mod._rel_to_stage_dir("/root/.config/app.toml", "/root") == ".config/app.toml"


async def test_sdk_direct_stage_dir_transfers_and_assembly_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    stage_dir = tmp_path / "root"
    stage_dir.mkdir()
    monkeypatch.setattr(xfer_mod, "_XFER_STAGE_DIR", str(stage_dir))
    monkeypatch.setattr(xfer_mod, "_XFER_PART_BYTES", 8)

    writes: list[tuple[str, bytes]] = []
    execs: list[str] = []
    fail_cat = False
    hv = capsem.Hypervisor("http://127.0.0.1:1", "t")
    files_cls = type(hv.vm(id="probe").files)

    async def fake_create(self: Any, **kwargs: Any) -> Any:
        del kwargs
        return self.vm(id="vm-direct")

    async def fake_write_file(self: Any, path: str, data: bytes) -> None:
        del self
        writes.append((path, data))

    async def fake_read_file(self: Any, path: str) -> bytes:
        del self
        assert path == "direct/file.bin"
        return b"direct-bytes"

    async def fake_exec(
        self: Any, command: str, *, timeout_secs: int | None = None, target: Any = None
    ) -> Any:
        del self, timeout_secs, target
        execs.append(command)
        if fail_cat and "cat " in command:
            return exec_response(1, stderr="disk full")
        return exec_response(0)

    monkeypatch.setattr(capsem.Hypervisor, "create", fake_create)
    monkeypatch.setattr(files_cls, "write", fake_write_file)
    monkeypatch.setattr(files_cls, "read", fake_read_file)
    monkeypatch.setattr(capsem.VM, "exec", fake_exec)

    sdk_ctrl = SdkCapsemController(hypervisor=hv)
    try:
        vid = await sdk_ctrl.start_vm(cpu_count=1, ram_gb=1)
        target = str(stage_dir / "direct" / "file.bin")
        await sdk_ctrl.upload_to_vm(vid, target, b"abc")
        assert writes == [("direct/file.bin", b"abc")]
        assert await sdk_ctrl.download_from_vm(vid, target) == b"direct-bytes"
        assert await sdk_ctrl.download_from_vm(vid, target, max_bytes=5) == b"direct"
        assert execs == []
        fail_cat = True
        with pytest.raises(RuntimeError, match="disk full"):
            await sdk_ctrl.upload_to_vm(vid, "/dest", b"abc")
    finally:
        await sdk_ctrl.close()


async def test_staged_transfers_suppress_finally_cleanup_error_and_catch_403_413(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(xfer_mod, "_XFER_STAGE_DIR", "/root")
    monkeypatch.setattr(xfer_mod, "_XFER_PART_BYTES", 4)

    class FailingCleanupController(Scripted):
        async def exec_in_vm(
            self, vm_id: str, command: str, *, timeout: int = 120
        ) -> CommandResult:
            if command.startswith(("rm -rf ", "rm -f ")):
                raise RuntimeError("VM gone during finally cleanup")
            return await super().exec_in_vm(vm_id, command, timeout=timeout)

    written_parts: list[tuple[str, bytes]] = []
    read_parts: list[str] = []

    class _StubFiles:
        def __init__(self, *, fail_write: bool = False) -> None:
            self.fail_write = fail_write

        async def write(self, rel: str, data: bytes) -> None:
            if self.fail_write:
                raise RuntimeError("part write failed")
            if rel == "symlink_dest.bin":
                raise HttpError(403, "Rejected direct path")
            written_parts.append((rel, data))

        async def read(self, rel: str) -> bytes:
            if rel == "missing.bin":
                raise HttpError(404, f"Not found: /root/run-413/{rel}")
            if rel == "denied.bin":
                raise RuntimeError(f"permission denied reading /root/413/{rel}")
            if rel == "large.bin":
                raise HttpError(413, "Payload Too Large")
            if rel == "symlink.bin":
                raise HttpError(403, "Forbidden: symlink")
            read_parts.append(rel)
            return b"abcd"

    stub = _StubFiles()
    ctrl_ok_up = Scripted()
    await xfer_mod._staged_upload(ctrl_ok_up, stub, "vm-1", "/opt/out.bin", b"0123456789")
    assert len(ctrl_ok_up.commands) == 1
    assert "cat /root/.capsem-xfer-" in ctrl_ok_up.commands[0]
    assert "__ec=$?; rm -rf /root/.capsem-xfer-" in ctrl_ok_up.commands[0]

    await ctrl_ok_up.upload_to_vm("vm-1", "/opt/out.bin", b"data")
    assert ctrl_ok_up.uploads["/opt/out.bin"] == b"data"
    assert await ctrl_ok_up.download_from_vm("vm-1", "/opt/out.bin") == b"downloaded"

    written_parts.clear()
    ctrl_403_up = Scripted()
    await xfer_mod._staged_upload(ctrl_403_up, stub, "vm-1", "/root/symlink_dest.bin", b"hi")
    await xfer_mod._staged_upload(ctrl_403_up, stub, "vm-1", "/root/a:b.txt", b"hi")
    assert len(written_parts) == 2
    assert any("cat /root/.capsem-xfer-" in c for c in ctrl_403_up.commands)

    ctrl_up = FailingCleanupController([("cat ", fail(stderr="assembly failed"))])
    with pytest.raises(RuntimeError, match="assembly failed"):
        await xfer_mod._staged_upload(ctrl_up, stub, "vm-1", "/opt/out.bin", b"0123456789")
    with pytest.raises(RuntimeError, match="part write failed"):
        await xfer_mod._staged_upload(
            ctrl_up, _StubFiles(fail_write=True), "vm-1", "/opt/out.bin", b"0123456789"
        )

    ctrl_down = FailingCleanupController([("split ", fail(stderr="split failed"))])
    with pytest.raises(RuntimeError, match="split failed"):
        await xfer_mod._staged_download(ctrl_down, stub, "vm-1", "/opt/in.bin")

    ctrl_split = Scripted([("split -b", ok("part.000000\n"))])
    with pytest.raises(HttpError, match="Not found"):
        await xfer_mod._staged_download(ctrl_split, stub, "vm-1", "/root/missing.bin")
    with pytest.raises(RuntimeError, match="permission denied"):
        await xfer_mod._staged_download(ctrl_split, stub, "vm-1", "/root/denied.bin")
    assert ctrl_split.commands == []

    assert await xfer_mod._staged_download(ctrl_split, stub, "vm-1", "/root/large.bin") == b"abcd"
    assert any("split -b" in c for c in ctrl_split.commands)
    assert await xfer_mod._staged_download(ctrl_split, stub, "vm-1", "/root/symlink.bin") == b"abcd"
    assert await xfer_mod._staged_download(ctrl_split, stub, "vm-1", "/root/a:b.txt") == b"abcd"

    read_parts.clear()
    ctrl_bounded = Scripted([("split -b", ok("part.000000\npart.000001\npart.000002\n"))])
    res_bounded = await xfer_mod._staged_download(
        ctrl_bounded, stub, "vm-1", "/opt/growing.bin", max_bytes=5
    )
    assert res_bounded == b"abcdab" and len(read_parts) == 2
    assert any(
        "set -o pipefail; [ -f /opt/growing.bin ]" in c
        and "head -c 6 -- /opt/growing.bin | split -b" in c
        for c in ctrl_bounded.commands
    )


class _LocalSdkVM:
    def __init__(self, root: Path) -> None:
        self.id = "vm-local-sdk"
        self.root = root
        self.writes: list[int] = []
        self.exec_count = 0
        self.files = self

    async def write(self, path: str, data: bytes) -> None:
        _server_check_rel_path(path)
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        self.writes.append(len(data))

    async def read(self, path: str) -> bytes:
        _server_check_rel_path(path)
        return (self.root / path).read_bytes()

    async def exec(
        self, command: str, *, timeout_secs: int | None = None, target: Any = None
    ) -> object:
        del target
        self.exec_count += 1
        proc = sp.run(
            ["bash", "-c", command], capture_output=True, timeout=timeout_secs, check=False
        )
        b64_enc = ExecOutputEncoding.BASE64
        return SimpleNamespace(
            exit_code=proc.returncode,
            stdout=ExecOutput(data=b64.b64encode(proc.stdout).decode(), encoding=b64_enc),
            stderr=ExecOutput(data=b64.b64encode(proc.stderr).decode(), encoding=b64_enc),
            truncated=False,
        )

    async def delete(self) -> None:
        return None


async def test_sdk_controller_file_transfer_self_check(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Writes and reads go through Files API parts, not base64 exec chunks."""
    stage_root = tmp_path / "root"
    stage_root.mkdir()
    monkeypatch.setattr(xfer_mod, "_XFER_STAGE_DIR", str(stage_root))
    monkeypatch.setattr(xfer_mod, "_XFER_PART_BYTES", 1024)
    local_vm = _LocalSdkVM(stage_root)

    async def _create(**_: Any) -> _LocalSdkVM:
        return local_vm

    async def _close() -> None:
        return None

    hv = SimpleNamespace(vm=lambda **_: local_vm, create=_create, close=_close)
    ctrl = SdkCapsemController(hypervisor=cast(Any, hv))
    work = tmp_path / "work"
    work.mkdir()
    try:
        vid = await ctrl.start_vm(cpu_count=1, ram_gb=1)
        data = os.urandom(5000)
        await ctrl.upload_to_vm(vid, str(work / "sub" / "blob.bin"), data)
        assert (work / "sub" / "blob.bin").read_bytes() == data
        assert local_vm.writes == [1024, 1024, 1024, 1024, 904]
        assert await ctrl.download_from_vm(vid, str(work / "sub" / "blob.bin")) == data
        await ctrl.upload_to_vm(vid, str(work / "empty.bin"), b"")
        assert await ctrl.download_from_vm(vid, str(work / "empty.bin")) == b""
        assert list(stage_root.iterdir()) == []
        colon_path = stage_root / "a:b.txt"
        await ctrl.upload_to_vm(vid, str(colon_path), b"colon-ok")
        assert colon_path.read_bytes() == b"colon-ok"
        assert await ctrl.download_from_vm(vid, str(colon_path)) == b"colon-ok"
        colon_path.unlink()
        assert list(stage_root.iterdir()) == []

        env = CapsemSandboxEnvironment(vm_id=vid, controller=ctrl, working_dir=str(work))
        before = local_vm.exec_count
        big = os.urandom(200_000)
        await env.write_file("big.bin", big)
        assert await env.read_file("big.bin", text=False) == big
        assert local_vm.exec_count - before < 12
        skip_tests = {
            "test_read_and_write_large_file_binary",
            "test_exec_input_large",
            "test_exec_as_user",
        } | _host_timeout_skips()
        results = await _run_inspect_self_check(env, skip=skip_tests)
        failures = {k: v for k, v in results.items() if v is not True}
        assert failures == {}, f"self_check via SDK transfer failed: {failures}"
    finally:
        await ctrl.close()
