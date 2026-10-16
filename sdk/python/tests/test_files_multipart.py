"""Tests for VM.files.upload and VM.files.download multipart and out-of-workspace transfers."""

from __future__ import annotations

import asyncio
import base64
import os
import subprocess
from pathlib import Path
from typing import Any, cast

import capsem._resources as res_mod
import pytest
from capsem import VM, ExecResult, HttpError, Hypervisor, models
from pydantic import ValidationError

from .facade_gateway import gateway


def _exec_result(exit_code: int, stdout: bytes = b"", stderr: bytes = b"") -> ExecResult:
    return ExecResult(
        exit_code=exit_code,
        stdout=models.ExecOutput(
            encoding=models.ExecOutputEncoding.BASE64,
            data=base64.b64encode(stdout).decode(),
        ),
        stderr=models.ExecOutput(
            encoding=models.ExecOutputEncoding.BASE64,
            data=base64.b64encode(stderr).decode(),
        ),
    )


def _run_bash(command: str, timeout_secs: int | None) -> ExecResult:
    proc = subprocess.run(
        ["bash", "-c", command],
        capture_output=True,
        timeout=timeout_secs,
        check=False,
    )
    return _exec_result(proc.returncode, proc.stdout, proc.stderr)


def test_rel_to_stage_dir_path_validation() -> None:
    for invalid in (
        "/workspace/../etc/passwd",
        "/workspace/sub/../ok.txt",
        "/etc/passwd",
        "/workspace",
        "/workspace/a:b.txt",
        "/workspace/a@b.txt",
        "/workspace/a+b.txt",
        "/workspace/with space.txt",
    ):
        assert res_mod._rel_to_stage_dir(invalid, "/workspace") is None
    assert res_mod._rel_to_stage_dir("/workspace/sub/ok_1.txt", "/workspace") == "sub/ok_1.txt"
    assert res_mod._rel_to_stage_dir("/root/.bashrc", "/root") == ".bashrc"
    assert res_mod._rel_to_stage_dir("/root/.config/app.toml", "/root") == ".config/app.toml"
    assert res_mod._rel_to_stage_dir("/sub/ok.txt", "/") == "sub/ok.txt"


def test_transfer_validation_rejects_bad_arguments_before_network() -> None:
    async def run() -> None:
        async with gateway() as (url, state), VM(url, "token", name="named") as vm:
            for bad_path in ("", "/", "///"):
                with pytest.raises(ValueError, match="path must name a file"):
                    await vm.files.upload(bad_path, b"x")
                with pytest.raises(ValueError, match="path must name a file"):
                    await vm.files.download(bad_path)
            with pytest.raises(ValidationError):
                await vm.files.upload(cast(str, 123), b"x")
            with pytest.raises(TypeError, match="data must be bytes"):
                await vm.files.upload("/root/a.txt", cast(bytes, "text"))
            for bad_max in (-1, True, "10"):
                with pytest.raises(ValueError, match="max_bytes"):
                    await vm.files.download("/root/a.txt", max_bytes=cast(int, bad_max))
            with pytest.raises(ValidationError):
                await vm.files.upload(
                    "/root/a.txt", b"x", target=cast(models.ExecTarget, "invalid")
                )
            with pytest.raises(ValidationError):
                await vm.files.download(
                    "/root/a.txt", stage_dir=cast(str, 123)
                )
            with pytest.raises(ValueError, match="stage_dir must be an absolute POSIX path"):
                await vm.files.upload("/root/a.txt", b"x", stage_dir="relative/dir")
            assert state.requests == []
            assert vm.id is None

    asyncio.run(run())


def test_direct_fast_path_for_vm_and_container_workspaces() -> None:
    async def run() -> None:
        async with gateway() as (url, state), Hypervisor(url, "token") as hv:
            vm = hv.vm(id="vm-0")
            await vm.files.upload("direct/rel.bin", b"rel-bytes")
            assert await vm.files.download("/root/direct/rel.bin") == b"rel-bytes"
            assert await vm.files.download("direct/rel.bin", max_bytes=9) == b"rel-bytes"
            with pytest.raises(ValueError, match="exceeds max_bytes=3"):
                await vm.files.download("direct/rel.bin", max_bytes=3)

            container_vm = await hv.create(image="docker://busybox:latest")
            before = len(state.requests)
            await container_vm.files.upload("/workspace/c.bin", b"container-bytes")
            assert await container_vm.files.download("c.bin") == b"container-bytes"
            assert not any(
                path.endswith("/exec") for _, path, _ in state.requests[before:]
            )
            with pytest.raises(HttpError) as err:
                await vm.files.download("/root/missing.bin")
            assert err.value.status == 404

    asyncio.run(run())


def test_multipart_and_outside_workspace_roundtrip(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    stage_root = tmp_path / "root"
    stage_root.mkdir()
    work = tmp_path / "work"
    work.mkdir()
    monkeypatch.setattr(res_mod, "_XFER_STAGE_DIR", str(stage_root))
    monkeypatch.setattr(res_mod, "_XFER_PART_BYTES", 1024)

    writes: list[int] = []

    async def fake_write(self: Any, path: str, data: bytes, *, exact: bool = False) -> Any:
        del self, exact
        if ".." in path or any(ch in path for ch in (":", "@", "+")):
            raise HttpError(400, "invalid path")
        target = stage_root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        writes.append(len(data))
        return models.UploadResponse(success=True, size=len(data), vm_path=str(target))

    async def fake_read(self: Any, path: str, *, exact: bool = False) -> bytes:
        del self, exact
        if ".." in path or any(ch in path for ch in (":", "@", "+")):
            raise HttpError(400, "invalid path")
        target = stage_root / path
        if not target.is_file():
            raise HttpError(404, "not found")
        data = target.read_bytes()
        if len(data) > 1024:
            raise HttpError(413, "file too large")
        return data

    async def fake_exec(
        self: Any,
        command: str,
        *,
        timeout_secs: int | None = None,
        target: models.ExecTarget | None = None,
    ) -> ExecResult:
        del self, target
        return await asyncio.to_thread(_run_bash, command, timeout_secs)

    monkeypatch.setattr(res_mod.Files, "write", fake_write)
    monkeypatch.setattr(res_mod.Files, "read", fake_read)
    monkeypatch.setattr(VM, "exec", fake_exec)

    async def run() -> None:
        async with VM("http://127.0.0.1:1", "token", id="vm-local") as vm:
            blob = os.urandom(3500)
            dest = str(work / "sub" / "blob.bin")
            await vm.files.upload(dest, blob)
            assert (work / "sub" / "blob.bin").read_bytes() == blob
            assert writes == [1024, 1024, 1024, 428]
            assert await vm.files.download(dest) == blob
            assert await vm.files.download(dest, max_bytes=len(blob)) == blob
            with pytest.raises(ValueError, match="exceeds max_bytes=1500"):
                await vm.files.download(dest, max_bytes=1500)

            # Large file inside stage_dir triggers 413 on direct read and falls back to split.
            in_stage_big = str(stage_root / "big_in_stage.bin")
            await vm.files.upload(in_stage_big, blob)
            assert await vm.files.download(in_stage_big) == blob

            # Empty file outside workspace and special-character filename inside stage_root.
            empty_dest = str(work / "empty.bin")
            await vm.files.upload(empty_dest, b"")
            assert await vm.files.download(empty_dest) == b""
            colon_dest = str(stage_root / "a:b.txt")
            await vm.files.upload(colon_dest, b"colon-ok")
            assert await vm.files.download(colon_dest) == b"colon-ok"
            (stage_root / "a:b.txt").unlink()
            (stage_root / "big_in_stage.bin").unlink()
            assert list(stage_root.iterdir()) == []

    asyncio.run(run())


def test_staged_error_paths_and_cleanup_suppression(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(res_mod, "_XFER_PART_BYTES", 4)
    commands: list[tuple[str, models.ExecTarget | None]] = []
    fail_mode = ""

    async def fake_write(self: Any, path: str, data: bytes, *, exact: bool = False) -> Any:
        del self, data, exact
        if path == "forbidden.bin":
            raise HttpError(403, "symlink refused")
        if path == "server_err.bin":
            raise HttpError(500, "disk error")
        if fail_mode == "write_part":
            raise RuntimeError("part write failed")
        return models.UploadResponse(success=True, size=2, vm_path=f"/root/{path}")

    async def fake_read(self: Any, path: str, *, exact: bool = False) -> bytes:
        del self, exact
        if path == "forbidden.bin":
            raise HttpError(403, "symlink refused")
        return b"abcd"

    async def fake_exec(
        self: Any,
        command: str,
        *,
        timeout_secs: int | None = None,
        target: models.ExecTarget | None = None,
    ) -> ExecResult:
        del self, timeout_secs
        commands.append((command, target))
        if command.startswith("rm -rf "):
            raise RuntimeError("VM gone during finally cleanup")
        if fail_mode == "empty":
            return _exec_result(1, stdout=b"mkdir failed")
        if fail_mode == "cat" and "cat " in command:
            return _exec_result(1, stderr=b"assembly failed")
        if fail_mode == "split" and "split " in command:
            return _exec_result(1, stderr=b"split failed")
        return _exec_result(0, stdout=b"part.000000\npart.000001\n")

    monkeypatch.setattr(res_mod.Files, "write", fake_write)
    monkeypatch.setattr(res_mod.Files, "read", fake_read)
    monkeypatch.setattr(VM, "exec", fake_exec)

    async def run() -> None:
        nonlocal fail_mode
        async with VM("http://127.0.0.1:1", "token", id="vm-err") as vm:
            with pytest.raises(HttpError) as err500:
                await vm.files.upload("/root/server_err.bin", b"hi")
            assert err500.value.status == 500

            # 403 on direct write/read falls back to staged transfer; WORKLOAD target uses /workspace.
            await vm.files.upload(
                "forbidden.bin", b"hi", target=models.ExecTarget.WORKLOAD
            )
            assert "/workspace/.capsem-xfer-" in commands[-1][0]
            assert commands[-1][1] is models.ExecTarget.WORKLOAD
            assert await vm.files.download("/root/forbidden.bin", stage_dir="/root") == b"abcdabcd"

            fail_mode = "empty"
            with pytest.raises(RuntimeError, match="mkdir failed"):
                await vm.files.upload("/opt/empty.bin", b"")

            fail_mode = "cat"
            with pytest.raises(RuntimeError, match="assembly failed"):
                await vm.files.upload("/opt/out.bin", b"01234567")

            fail_mode = "write_part"
            with pytest.raises(RuntimeError, match="part write failed"):
                await vm.files.upload("/opt/out.bin", b"01234567")

            fail_mode = "split"
            with pytest.raises(RuntimeError, match="split failed"):
                await vm.files.download("/opt/in.bin")

    asyncio.run(run())
