"""Inspect sandbox tools resolution, baking, and container/OCI file transfer tests."""

from __future__ import annotations

import asyncio
import gzip
import io
import logging
import os
import tarfile
from pathlib import Path
from types import SimpleNamespace
from typing import Any, cast

import inspect_capsem._controller as ctrl_mod
import inspect_capsem._tools as tools_mod
import inspect_capsem.sandbox as sb
import pytest
from inspect_capsem import (
    INSPECT_SANDBOX_TOOLS_GUEST_PATH,
    CommandResult,
    SdkCapsemController,
    bake_sandbox_tools_into_controller,
    resolve_inspect_sandbox_tools_host_binary,
)
from inspect_capsem._tools import INSPECT_SANDBOX_TOOLS_ONEDIR_PATH

from .conftest import LocalFakeCapsemController, Scripted, env_for, fail, ok


def found_name() -> str:
    found = resolve_inspect_sandbox_tools_host_binary()
    assert found is not None
    return found.name


def test_resolve_tools_binary_candidates(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    import inspect_ai

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


def test_bake_via_workspace_and_transfer(
    gz_tools: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("HOME", str(tmp_path))
    xfer_hit = Scripted()
    assert asyncio.run(
        bake_sandbox_tools_into_controller(xfer_hit, "vm", host_binary_path=gz_tools)
    )
    assert len(xfer_hit.uploads) == 0 and len(xfer_hit.commands) == 1

    xfer = Scripted([("test -x", fail())])
    assert asyncio.run(bake_sandbox_tools_into_controller(xfer, "vm", host_binary_path=gz_tools))
    assert xfer.uploads[sb.INSPECT_SANDBOX_TOOLS_GUEST_PATH] == gz_tools.read_bytes()
    assert len(xfer.uploads) == 1 and len(xfer.commands) == 2

    xfer = Scripted([("test -x", fail())])
    assert asyncio.run(
        bake_sandbox_tools_into_controller(xfer, "vm", container_id="c1", host_binary_path=gz_tools)
    )
    assert len(xfer.uploads) == 1 and len(xfer.commands) == 2
    assert any(c.startswith("docker exec -i -u 0 c1 ") for c in xfer.commands)

    xfer_oci = Scripted([("test -x", fail())])
    assert asyncio.run(
        bake_sandbox_tools_into_controller(
            xfer_oci, "vm", container_id="workload", host_binary_path=gz_tools
        )
    )
    assert len(xfer_oci.uploads) == 1 and len(xfer_oci.commands) == 2
    assert any(c.startswith(f"{tools_mod._OCI_RUNC_EXEC_ROOT} ") for c in xfer_oci.commands)
    xfer_fail = Scripted([("test -x", fail()), ("chmod 700", fail(stderr="ro fs"))])
    assert not asyncio.run(
        bake_sandbox_tools_into_controller(xfer_fail, "vm", host_binary_path=gz_tools)
    )

    # A host-side persistent session dir is no longer used for staging.
    pws = tmp_path / ".capsem" / "run" / "persistent" / "vm" / "workspace"
    pws.mkdir(parents=True)
    xfer = Scripted([("test -x", fail())])
    assert asyncio.run(bake_sandbox_tools_into_controller(xfer, "vm", host_binary_path=gz_tools))
    assert not any("/root/.capsem_stage_tools" in c for c in xfer.commands)
    assert xfer.uploads[sb.INSPECT_SANDBOX_TOOLS_GUEST_PATH] == gz_tools.read_bytes()


def test_bake_without_workspace_uses_files_api_parts(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The ~14 MiB tools binary must reach an ephemeral VM in parts under the chunk cap."""
    from capsem.models import ExecOutput, ExecOutputEncoding

    monkeypatch.setattr(ctrl_mod, "_XFER_PART_BYTES", 4)
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

        async def exec(self, command: str, *, timeout_secs: int | None = None) -> Any:
            del timeout_secs
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

    async def _run() -> None:
        ctrl = SdkCapsemController(hypervisor=Hv())
        try:
            vid = await ctrl.start_vm(template="code", cpu_count=1, ram_gb=1)
            assert str(created[0].get("name", "")).startswith("inspect-capsem-")
            assert await bake_sandbox_tools_into_controller(ctrl, vid, host_binary_path=tools)
        finally:
            await ctrl.close()

    asyncio.run(_run())
    assert writes == [b"0123", b"4567", b"89"]
    assert any(
        c.startswith("mkdir -p") and f"/* > {sb.INSPECT_SANDBOX_TOOLS_GUEST_PATH}" in c
        for c in commands
    )


def test_bake_failures(gz_tools: Path, tmp_path: Path) -> None:
    assert not asyncio.run(
        bake_sandbox_tools_into_controller(Scripted(), "vm", host_binary_path=tmp_path / "no")
    )
    assert asyncio.run(
        bake_sandbox_tools_into_controller(Scripted(), "vm", host_binary_path=gz_tools)
    )
    ctrl = Scripted([("test -x", fail()), ("chmod 700", fail())])
    assert not asyncio.run(
        bake_sandbox_tools_into_controller(ctrl, "vm", host_binary_path=gz_tools)
    )
    plain = tmp_path / "plain"
    plain.write_bytes(b"not gzip")
    ctrl = Scripted([("test -x", fail())])
    assert asyncio.run(bake_sandbox_tools_into_controller(ctrl, "vm", host_binary_path=plain))
    assert not any("tar -xzf" in c for c in ctrl.commands)


def test_container_transfer_failures() -> None:
    ctrl = Scripted([("docker exec -i", fail(stderr="no space"))])
    with pytest.raises(RuntimeError, match="no space"):
        asyncio.run(tools_mod._upload_via_transfer(ctrl, "vm", "c1", "/a/b", b"x"))
    assert any(c.startswith("rm -f /root/.capsem-xfer-file-") for c in ctrl.commands)
    ctrl = Scripted([("docker exec -u 0 c1 cat", fail(stderr="denied"))])
    with pytest.raises(PermissionError, match="denied"):
        asyncio.run(tools_mod._download_via_transfer(ctrl, "vm", "c1", "/a/b"))
    ctrl = Scripted()
    assert asyncio.run(tools_mod._download_via_transfer(ctrl, "vm", "c1", "/a/b")) == b"downloaded"


def test_transfer_controller_staging_in_container_and_oci_workload() -> None:
    ctrl = Scripted()
    env = env_for(ctrl, container_id="c1", execution_mode="container")
    asyncio.run(env._stage_bytes_to_guest(b"payload", "/data/x"))
    assert any(v == b"payload" for v in ctrl.uploads.values())
    assert asyncio.run(env._fetch_guest_file_bytes("/data/x")) == b"downloaded"

    # OCI workload container: /workspace/<rel> maps directly to /root/<rel> without runc exec!
    ctrl_oci = Scripted()
    ctrl_oci.files["/root/sub/file.txt"] = b"from-root-mount"
    env_oci = env_for(ctrl_oci, container_id="workload", execution_mode="container")
    asyncio.run(env_oci._stage_bytes_to_guest(b"direct-ws", "/workspace/sub/file.txt"))
    assert ctrl_oci.uploads["/root/sub/file.txt"] == b"direct-ws"
    assert ctrl_oci.commands == []
    assert asyncio.run(env_oci._fetch_guest_file_bytes("/workspace/sub/file.txt")) == (
        b"from-root-mount"
    )
    assert ctrl_oci.commands == []

    # Non-/workspace path inside OCI workload container stages via /root and runc exec -u 0:0.
    asyncio.run(env_oci._stage_bytes_to_guest(b"in-etc", "/etc/custom.conf"))
    assert any(
        tools_mod._OCI_RUNC_EXEC_ROOT in c and "/etc/custom.conf" in c for c in ctrl_oci.commands
    )
    assert asyncio.run(env_oci._fetch_guest_file_bytes("/etc/custom.conf")) == b"downloaded"

    # Paths with characters outside ^[A-Za-z0-9._\-/]+$ or '..' traversal are rejected by
    # _rel_to_stage_dir, while dot-prefixed files under stage_dir are accepted.
    for unsafe_rel in (
        "/workspace/a:b.txt",
        "/workspace/a@b.txt",
        "/workspace/a+b.txt",
        "/workspace/../etc/passwd",
        "/workspace/a..b",
    ):
        assert ctrl_mod._rel_to_stage_dir(unsafe_rel, "/workspace") is None
    assert ctrl_mod._rel_to_stage_dir("/root/a..b", "/root") is None
    assert (
        ctrl_mod._rel_to_stage_dir("/workspace/sub/ok_file-1.txt", "/workspace")
        == "sub/ok_file-1.txt"
    )
    assert ctrl_mod._rel_to_stage_dir("/root/.bashrc", "/root") == ".bashrc"
    assert ctrl_mod._rel_to_stage_dir("/root/.config/app.toml", "/root") == ".config/app.toml"


def test_container_write_file_chowns_to_nonroot_user(
    tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    """Container write_file chowns to cfg.user > .Config.User > /proc/1 and warns on failure."""
    import subprocess as sp

    from inspect_capsem._tools import _CHOWN_WARN_SENTINEL, _chown_to_container_user_snippet

    # 1. Verify the shell snippet directly with a fake chown binary in PATH.
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    chown_log = tmp_path / "chown.log"
    fake_chown = bin_dir / "chown"
    fake_chown.write_text(f'#!/bin/sh\necho "$@" >> "{chown_log}"\n')
    fake_chown.chmod(0o755)
    env = {**os.environ, "PATH": f"{bin_dir}:{os.environ.get('PATH', '/usr/bin:/bin')}"}

    # Explicit cfg.user ("developer") wins over image .Config.User ("imageuser").
    snippet_user = _chown_to_container_user_snippet("'/home/dev/file.py'", "developer")
    sp.run(["/bin/sh", "-c", snippet_user, "sh", "imageuser"], env=env, check=True)
    assert chown_log.read_text().strip() == "developer: /home/dev/file.py"
    chown_log.unlink()

    # When cfg.user is empty, image .Config.User ($1) is used.
    snippet_uid_gid = _chown_to_container_user_snippet("'/home/dev/file.py'", "")
    sp.run(["/bin/sh", "-c", snippet_uid_gid, "sh", "1000:1000"], env=env, check=True)
    assert chown_log.read_text().strip() == "1000:1000 /home/dev/file.py"
    chown_log.unlink()

    # Root user should NOT invoke chown.
    snippet_root = _chown_to_container_user_snippet("'/root/file.py'", "root")
    sp.run(["/bin/sh", "-c", snippet_root, "sh", "1000:1000"], env=env, check=True)
    assert not chown_log.exists()

    # 2. Verify container write_file via transfer emits the chown logic.
    ctrl = Scripted([("", ok())])
    sb_env = env_for(ctrl, container_id="app", execution_mode="container", user="developer")
    asyncio.run(sb_env.write_file("src/solution.py", "print(1)\n"))
    joined = "\n".join(ctrl.commands)
    assert "docker inspect -f '{{.Config.User}}' app" in joined
    assert "chown " in joined
    assert "developer" in joined

    # 3. Verify OCI rootfs container mode ("workload") runs the snippet inside workload via capsem-chroot.
    ctrl_oci = Scripted([("", ok())])
    sb_oci = env_for(
        ctrl_oci,
        container_id=tools_mod._OCI_WORKLOAD_CONTAINER_ID,
        execution_mode="container",
        user="1000:1000",
    )
    asyncio.run(sb_oci.write_file("/workspace/fix.py", "x = 1\n"))
    joined_oci = "\n".join(ctrl_oci.commands)
    assert "/proc/1" in joined_oci
    assert "chown " in joined_oci
    assert "1000:1000" in joined_oci

    # 4. Verify non-zero chown logs a warning across transfer and OCI /workspace paths.
    caplog.clear()
    ctrl_warn_xfer = Scripted(
        [
            (
                _CHOWN_WARN_SENTINEL,
                CommandResult(0, "", f"chown: invalid user\n{_CHOWN_WARN_SENTINEL}\n"),
            )
        ]
    )
    sb_warn_xfer = env_for(
        ctrl_warn_xfer, container_id="app", execution_mode="container", user="baduser"
    )
    asyncio.run(sb_warn_xfer.write_file("a.py", "1"))
    assert "Failed to chown /workspace/a.py in container app" in caplog.text

    caplog.clear()
    ctrl_warn_oci = Scripted([("chown", fail(1, stderr="chown: invalid group"))])
    sb_warn_oci = env_for(
        ctrl_warn_oci,
        container_id=tools_mod._OCI_WORKLOAD_CONTAINER_ID,
        execution_mode="container",
        user="bad:group",
    )
    asyncio.run(sb_warn_oci.write_file("/workspace/b.py", "1"))
    assert "Failed to chown /workspace/b.py in container workload" in caplog.text


@pytest.mark.asyncio
async def test_resolve_and_bake_sandbox_tools_v29_and_warning(
    tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    """Resolves versioned inspect-sandbox-tools binary, extracts tar.gz, and warns on missing."""
    resolved = resolve_inspect_sandbox_tools_host_binary()
    assert resolved is not None
    assert resolved.is_file()
    assert "inspect-sandbox-tools-" in resolved.name

    controller = LocalFakeCapsemController(tmp_path, skip_bake=False)
    vid = await controller.start_vm(template="code", cpu_count=2, ram_gb=4)

    with caplog.at_level(logging.WARNING):
        ok_missing = await bake_sandbox_tools_into_controller(
            cast(Any, controller), vid, host_binary_path=tmp_path / "does-not-exist"
        )
    assert ok_missing is False
    assert any("Could not locate host inspect-sandbox-tools" in r.message for r in caplog.records)

    buf = io.BytesIO()
    with gzip.GzipFile(fileobj=buf, mode="wb") as gz, tarfile.open(fileobj=gz, mode="w") as tf:
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
