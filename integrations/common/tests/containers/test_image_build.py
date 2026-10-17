"""Unit and hermetic Docker frontend tests for `image_build`."""

from __future__ import annotations

import asyncio
import io
import json
import shutil
import subprocess
import tarfile
import time
from pathlib import Path
from typing import Any, cast

import pytest
from capsem import HttpError
from capsem_integrations._controller import SdkCapsemController
from capsem_integrations.containers import HostBuildGrant, resolve_direct_host_build
from capsem_integrations.containers import image_build as ib_mod
from capsem_integrations.containers import oci_ingest as ing_mod

from .test_oci_registry import _make_oci_tar


async def test_build_and_stage_lifecycle_cache_and_admission_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    cache_dir = tmp_path / "oci_cache"
    monkeypatch.setenv("CAPSEM_INSPECT_BUILD_CACHE_DIR", str(cache_dir))
    monkeypatch.setenv("CAPSEM_INSPECT_BUILD_REGISTRY_PORT", "0")
    monkeypatch.setenv("CAPSEM_INSPECT_BUILD_TIMEOUT", "15")
    assert ib_mod.configured_build_timeout() == 15.0
    ctx_dir = tmp_path / "app"
    ctx_dir.mkdir()
    df = ctx_dir / "Dockerfile"
    df.write_text("FROM alpine:3.20\n", encoding="utf-8")

    monkeypatch.setenv("CAPSEM_INSPECT_HOST_BUILD", "0")
    with pytest.raises(RuntimeError, match="CAPSEM_INSPECT_HOST_BUILD=1"):
        await ib_mod.build_and_stage_oci_image({"context": str(ctx_dir), "dockerfile": str(df)})
    with pytest.raises(RuntimeError, match="CAPSEM_INSPECT_HOST_BUILD=1"):
        ib_mod.run_host_build({"context": str(ctx_dir), "dockerfile": str(df)})
    monkeypatch.setenv("CAPSEM_INSPECT_HOST_BUILD", "1")
    monkeypatch.setenv("CAPSEM_INSPECT_ALLOWED_HOST_PATHS", str(tmp_path))

    real_which, real_run = ib_mod.shutil.which, subprocess.run
    monkeypatch.setattr(ib_mod.shutil, "which", lambda n: None if n == "docker" else real_which(n))
    with pytest.raises(RuntimeError, match="Docker CLI"):
        await ib_mod.build_and_stage_oci_image({"context": str(ctx_dir), "dockerfile": str(df)})

    calls: list[tuple[list[str], dict[str, str]]] = []
    fail_mode: str | None = None

    def fake_run(cmd: list[str], **kw: Any) -> subprocess.CompletedProcess[str]:
        if cmd and "openssl" in cmd[0]:
            return real_run(cmd, **kw)
        calls.append((cmd, dict(kw.get("env") or {})))
        if fail_mode == "timeout":
            raise subprocess.TimeoutExpired(cmd, kw.get("timeout", 15.0))
        if fail_mode and fail_mode in cmd:
            return subprocess.CompletedProcess(cmd, 1, "", f"{fail_mode} failed")
        if "build" in cmd:
            time.sleep(0.02)
        if "save" in cmd:
            _make_oci_tar(Path(cmd[cmd.index("-o") + 1]))
        return subprocess.CompletedProcess(cmd, 0, "", "")

    monkeypatch.setattr(
        ib_mod.shutil, "which", lambda n: "/usr/bin/docker" if n == "docker" else real_which(n)
    )
    monkeypatch.setattr(ib_mod.subprocess, "run", fake_run)

    with pytest.raises(FileNotFoundError, match="Dockerfile not found"):
        ib_mod.run_host_build({"context": str(ctx_dir), "dockerfile": str(ctx_dir / "Missing")})

    dcfg_dir = tmp_path / "dcfg"
    dcfg_dir.mkdir()
    spec = {
        "context": str(ctx_dir),
        "dockerfile": str(df),
        "target": "s1",
        "args": {"K": "V"},
        "docker_config_dir": str(dcfg_dir),
    }
    for mode, match in (
        ("build", "Host 'docker build' failed"),
        ("save", "Host 'docker image save' failed"),
        ("timeout", "CAPSEM_INSPECT_BUILD_TIMEOUT"),
    ):
        fail_mode = mode
        with pytest.raises(RuntimeError, match=match):
            await ib_mod.build_and_stage_oci_image(spec)
    fail_mode = None
    calls.clear()

    # 4 concurrent cache-miss builds for the same spec execute `run_host_build` once
    results = await asyncio.gather(*(ib_mod.build_and_stage_oci_image(spec) for _ in range(4)))
    assert len(set(results)) == 1 and len(calls) == 3
    calls.clear()

    build_spec = resolve_direct_host_build({"dockerfile": str(df)})
    assert build_spec is not None
    image_ref, r_ca = await ib_mod.build_and_stage_oci_image(build_spec)
    assert len(calls) == 3
    build_cmd, _ = calls[0]
    assert "--network" in build_cmd and build_cmd[build_cmd.index("--network") + 1] == "none"
    assert "--config" in build_cmd and build_cmd[build_cmd.index("--config") + 1] != str(
        Path.home() / ".docker"
    )
    assert image_ref.startswith("127.0.0.1:") and "BEGIN CERTIFICATE" in r_ca
    await ib_mod.build_and_stage_oci_image(build_spec)
    assert len(calls) == 3

    # Corrupted layer triggers automatic rebuild
    m_hex = image_ref.split("@sha256:", 1)[1]
    blobs_dir = cache_dir / "blobs" / "sha256"
    m_doc = json.loads((blobs_dir / m_hex).read_bytes().decode("utf-8"))
    layer_hex = str(m_doc["layers"][0]["digest"]).removeprefix("sha256:")
    (blobs_dir / layer_hex).write_bytes(b"corrupted-layer-bytes")
    assert not ing_mod.verify_cached_manifest_blobs(blobs_dir, m_hex)
    await ib_mod.build_and_stage_oci_image(build_spec)
    assert len(calls) == 6
    (blobs_dir / m_hex).write_bytes(b"not-json")
    assert not ing_mod.verify_cached_manifest_blobs(blobs_dir, m_hex)

    class RejectingScope:
        async def __aenter__(self) -> Any:
            raise HttpError(403, "image source is not allowed")

        async def close(self) -> None:
            return

    class RejectingHypervisor:
        def managed(self, **kwargs: Any) -> Any:
            assert kwargs.get("registry") is not None
            return RejectingScope()

        async def close(self) -> None:
            return

    authority = image_ref.split("/", 1)[0]
    sdk_ctrl = SdkCapsemController(hypervisor=cast(Any, RejectingHypervisor()))
    with pytest.raises(RuntimeError, match=f'admit = \\["{authority}/inspect-capsem/build"\\]'):
        await sdk_ctrl.start_vm(cpu_count=2, ram_gb=4, image=image_ref, registry_ca_pem=r_ca)


@pytest.mark.requires_docker
def test_real_docker_frontend_offline_multistage_build(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    docker = shutil.which("docker")
    if docker is None:
        pytest.skip("docker CLI not installed")
        return
    if (
        subprocess.run([docker, "info"], capture_output=True, check=False, timeout=10).returncode
        != 0
    ):
        pytest.skip("docker daemon unavailable")
    from ..oci_workload_fixture import (
        _build_rootfs_tar_bytes,
        _extract_busybox_from_initrd,
        _find_initrd_path,
    )

    try:
        initrd = _find_initrd_path()
    except RuntimeError:
        matches = [
            m
            for root in (Path.home() / "capsem/cache/target/assets", Path.home() / ".capsem/assets")
            if root.is_dir()
            for m in root.rglob("initrd.img")
            if m.is_file()
        ]
        if not matches:
            pytest.skip("initrd busybox unavailable")
        initrd = matches[0]
    busybox = _extract_busybox_from_initrd(initrd)

    base_tag = "inspect-capsem-unit-base:local"
    subprocess.run(
        [docker, "image", "import", "-", base_tag],
        input=_build_rootfs_tar_bytes(busybox),
        check=True,
        capture_output=True,
        timeout=30,
    )
    ctx_dir = tmp_path / "proj"
    ctx_dir.mkdir()
    (ctx_dir / ".dockerignore").write_text("ignored.txt\n", encoding="utf-8")
    (ctx_dir / "ignored.txt").write_text("must-not-be-copied\n", encoding="utf-8")
    (ctx_dir / "kept.txt").write_text("kept-ok\n", encoding="utf-8")
    tar_buf = io.BytesIO()
    with tarfile.open(fileobj=tar_buf, mode="w") as tf:
        ti, data = tarfile.TarInfo(name="added.txt"), b"from-add-tar\n"
        ti.size, ti.mode = len(data), 0o644
        tf.addfile(ti, io.BytesIO(data))
    (ctx_dir / "archive.tar").write_bytes(tar_buf.getvalue())
    (ctx_dir / "Dockerfile").write_text(
        f"ARG BASE={base_tag}\nFROM ${{BASE}} AS builder\nARG MSG=default\n"
        'RUN echo "shell-${MSG}" > /built.txt\n'
        'RUN ["/bin/sh", "-c", "echo exec-ok >> /built.txt"]\n'
        "FROM scratch AS scratch_stage\nCOPY --from=builder /built.txt /built.txt\n"
        "FROM ${BASE} AS final\nWORKDIR /workspace/app\nENV APP_MODE=hermetic\n"
        "COPY --from=scratch_stage /built.txt /workspace/app/built.txt\n"
        "COPY --chown=1000:1000 kept.txt /workspace/app/kept.txt\n"
        "ADD archive.tar /workspace/app/unpacked/\n"
        'USER 1000:1000\nENTRYPOINT ["/bin/sh", "-c"]\nCMD ["sleep 3600"]\n',
        encoding="utf-8",
    )
    cache_dir = tmp_path / "oci_cache"
    monkeypatch.setenv("CAPSEM_INSPECT_HOST_BUILD", "1")
    monkeypatch.setenv("CAPSEM_INSPECT_ALLOWED_HOST_PATHS", str(ctx_dir))
    monkeypatch.setenv("CAPSEM_INSPECT_BUILD_CACHE_DIR", str(cache_dir))
    monkeypatch.setenv("CAPSEM_INSPECT_BUILD_REGISTRY_PORT", "0")
    grant = HostBuildGrant(allowed_contexts=(str(ctx_dir),), network="none")
    build_spec_multi = resolve_direct_host_build(
        {"build": {"context": str(ctx_dir), "target": "final", "args": {"MSG": "custom-arg"}}},
        host_build=grant,
    )
    assert build_spec_multi is not None
    image_ref, ca_pem = ib_mod.build_and_stage_oci_image_sync(build_spec_multi)
    assert image_ref.startswith("127.0.0.1:") and "BEGIN CERTIFICATE" in ca_pem
    m_hex = image_ref.split("@sha256:", 1)[1]
    blobs_dir = cache_dir / "blobs" / "sha256"
    m_doc = json.loads((blobs_dir / m_hex).read_bytes().decode("utf-8"))
    cfg_hex = str(m_doc["config"]["digest"]).removeprefix("sha256:")
    cfg_doc = json.loads((blobs_dir / cfg_hex).read_bytes().decode("utf-8"))
    assert cfg_doc["config"]["WorkingDir"] == "/workspace/app"
    assert cfg_doc["config"]["User"] == "1000:1000"
