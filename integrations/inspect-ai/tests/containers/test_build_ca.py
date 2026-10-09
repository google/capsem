"""Tests for PR 340b: CA bundle staging, rotation, NSS seeding, and BuildKit requirement."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import textwrap
from pathlib import Path
from typing import Any, cast

import pytest
from inspect_capsem import sandbox as sb_mod
from inspect_capsem.config import CapsemSandboxConfig
from inspect_capsem.containers import HostBuildGrant, prepare_oci_workload_container
from inspect_capsem.containers import build_ca as bca_mod
from inspect_capsem.containers import build_context as bc_mod
from inspect_capsem.containers import dockerfile_ca as ca_mod
from inspect_capsem.containers import image_build as ib_mod
from inspect_capsem.containers import oci_ingest as ing_mod

from ..helpers import Scripted
from .test_oci_registry import _make_oci_tar

_FAKE_DOCKER_SCRIPT = textwrap.dedent(
    """\
    #!/usr/bin/env python3
    import json, os, shutil, sys
    from pathlib import Path

    args = sys.argv[3:] if sys.argv[1:2] == ["--config"] else sys.argv[1:]
    if args[:1] == ["build"]:
        df = Path(args[args.index("-f") + 1]).read_text(encoding="utf-8")
        ctx = Path(args[-1])
        log_p = Path(os.environ["FAKE_DOCKER_LOG"])
        prev = json.loads(log_p.read_text(encoding="utf-8")) if log_p.is_file() else []
        log_p.write_text(json.dumps([*prev, df]), encoding="utf-8")
        rootfs = os.environ.get("FAKE_DOCKER_ROOTFS")
        if rootfs and (ctx / ".capsem-ca.crt").is_file():
            dst = Path(rootfs) / "usr/local/share/ca-certificates/capsem-ca.crt"
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ctx / ".capsem-ca.crt", dst)
    if args[:2] == ["image", "save"]:
        shutil.copyfile(os.environ["FAKE_DOCKER_TAR"], args[args.index("-o") + 1])
    """
)


def _install_fake_docker(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    bin_dir, tar_path = tmp_path / "bin", tmp_path / "template_oci.tar"
    bin_dir.mkdir(exist_ok=True)
    _make_oci_tar(tar_path)
    docker_bin = bin_dir / "docker"
    docker_bin.write_text(_FAKE_DOCKER_SCRIPT, encoding="utf-8")
    docker_bin.chmod(0o755)
    log_path = tmp_path / "docker_builds.json"
    monkeypatch.setenv("PATH", f"{bin_dir}:{os.environ.get('PATH', '')}")
    monkeypatch.setenv("FAKE_DOCKER_LOG", str(log_path))
    monkeypatch.setenv("FAKE_DOCKER_TAR", str(tar_path))
    return log_path


def _set_build_env(monkeypatch: pytest.MonkeyPatch, tmp_path: Path, **extra: str) -> None:
    base = {
        "HOST_BUILD": "1",
        "ALLOWED_HOST_PATHS": str(tmp_path),
        "BUILD_CACHE_DIR": str(tmp_path / "cache"),
        "BUILD_REGISTRY_PORT": "0",
        **extra,
    }
    for k, v in base.items():
        monkeypatch.setenv(f"CAPSEM_INSPECT_{k}", v)


def test_ca_grant_narrowing_and_context_staging(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    ctx = tmp_path / "ctx"
    ctx.mkdir()
    (ctx / "Dockerfile").write_text("FROM alpine:3.20\nRUN echo ok\n", encoding="utf-8")
    (ctx / ".dockerignore").write_text("*\n", encoding="utf-8")
    ca_file, bundle_file = tmp_path / "capsem-ca.crt", tmp_path / "ca-certificates.crt"
    ca_file.write_text("-----BEGIN CERTIFICATE-----\nCAPSEM-CA-1\n-----END CERTIFICATE-----\n")
    bundle_file.write_text("-----BEGIN CERTIFICATE-----\nPUBLIC-ROOT\n-----END CERTIFICATE-----\n")
    _set_build_env(monkeypatch, tmp_path, BUILD_NETWORK="default")

    b_arg = {"context": str(ctx)}
    with pytest.raises(TypeError, match="ca_pem must be a bool"):
        HostBuildGrant(ca_pem=cast(Any, str(ca_file)))
    with pytest.raises(TypeError, match="ca_pem must be a bool"):
        HostBuildGrant.from_value({"ca_pem": str(ca_file)})
    with pytest.raises(ValueError, match="Unknown HostBuildGrant fields"):
        HostBuildGrant.from_value({"ca_bundle": str(bundle_file)})

    monkeypatch.setenv("CAPSEM_INSPECT_BUILD_CA_PEM_FILE", str(ca_file))
    with pytest.raises(ValueError, match="CAPSEM_INSPECT_BUILD_CA_BUNDLE_FILE"):
        CapsemSandboxConfig(build=b_arg)

    opt_out = CapsemSandboxConfig(build=b_arg, host_build=HostBuildGrant(ca_pem=False))
    assert opt_out.build is not None and opt_out.build["ca_pem_file"] is None

    monkeypatch.setenv("CAPSEM_INSPECT_BUILD_CA_BUNDLE_FILE", str(bundle_file))
    cfg = CapsemSandboxConfig(build=b_arg, host_build=HostBuildGrant(ca_pem=True))
    assert cfg.build is not None
    assert cfg.build["ca_pem_file"] == str(ca_file.resolve())
    assert cfg.build["ca_bundle_file"] == str(bundle_file.resolve())

    staged = tmp_path / "staged"
    bca_mod.stage_capsem_ca_in_context(ctx, staged, cfg.build)
    assert (staged / ".capsem-ca.crt").read_text() == ca_file.read_text()
    merged = (staged / ".capsem-ca-bundle.crt").read_text()
    assert "PUBLIC-ROOT" in merged and "CAPSEM-CA-1" in merged
    dignore = (staged / ".dockerignore").read_text()
    assert "!.capsem-ca.crt" in dignore and "!.capsem-ca-bundle.crt" in dignore


async def test_ca_rotation_corrupt_blob_unlink_and_nss_seeding(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    log_path = _install_fake_docker(tmp_path, monkeypatch)
    cache_dir, ctx, rootfs = tmp_path / "cache", tmp_path / "app", tmp_path / "rootfs"
    ctx.mkdir()
    df, ca_file, bundle_file = ctx / "Dockerfile", tmp_path / "ca.crt", tmp_path / "bundle.crt"
    df.write_text("FROM alpine:3.20\nRUN echo hi\n", encoding="utf-8")
    key_out = ["-keyout", str(tmp_path / "ca.key"), "-out", str(ca_file)]
    openssl_cmd = ["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", *key_out]
    subprocess.run(
        [*openssl_cmd, "-days", "1", "-subj", "/CN=Capsem Test CA"],
        check=True,
        capture_output=True,
    )
    bundle_file.write_text("-----BEGIN CERTIFICATE-----\nBASE-ROOT\n-----END CERTIFICATE-----\n")
    monkeypatch.setenv("FAKE_DOCKER_ROOTFS", str(rootfs))
    _set_build_env(
        monkeypatch, tmp_path, BUILD_CA_PEM_FILE=str(ca_file), BUILD_CA_BUNDLE_FILE=str(bundle_file)
    )

    spec = CapsemSandboxConfig(build={"context": str(ctx)}).build
    assert spec is not None
    key1 = bc_mod.compute_build_cache_key(spec)
    ref1, _ = ib_mod.build_and_stage_oci_image_sync(spec)
    seen = json.loads(log_path.read_text(encoding="utf-8"))
    assert len(seen) == 1 and ".capsem-ca-bundle.crt" in seen[0] and "certutil" in seen[0]

    in_image_ca = rootfs / "usr/local/share/ca-certificates/capsem-ca.crt"
    assert in_image_ca.read_text(encoding="utf-8") == ca_file.read_text(encoding="utf-8")
    nss_home = tmp_path / "nss-home"
    nss_script = ca_mod._CA_NSS_SCRIPT.replace(ca_mod._IMAGE_CA, str(in_image_ca))
    subprocess.run(["sh", "-c", nss_script], env={**os.environ, "HOME": str(nss_home)}, check=True)
    if shutil.which("certutil"):
        cmd = ["certutil", "-d", f"sql:{nss_home}/.pki/nssdb", "-L", "-n", "capsem-ca"]
        listed = subprocess.run(cmd, check=True, capture_output=True, text=True)
        assert "Capsem Test CA" in listed.stdout

    m_hex = ref1.split("@sha256:", 1)[1]
    blobs_dir = cache_dir / "blobs" / "sha256"
    m_doc = json.loads((blobs_dir / m_hex).read_bytes().decode("utf-8"))
    layer_path = blobs_dir / str(m_doc["layers"][0]["digest"]).removeprefix("sha256:")
    layer_path.write_bytes(b"corrupted-bytes")
    assert not ing_mod.verify_cached_manifest_blobs(blobs_dir, m_hex) and not layer_path.exists()
    ib_mod.build_and_stage_oci_image_sync(spec)
    assert len(json.loads(log_path.read_text(encoding="utf-8"))) == 2 and layer_path.is_file()

    ca_file.write_text(ca_file.read_text() + "\n# rotated\n", encoding="utf-8")
    assert bc_mod.compute_build_cache_key(spec) != key1
    ib_mod.build_and_stage_oci_image_sync(spec)
    assert len(json.loads(log_path.read_text(encoding="utf-8"))) == 3

    ctrl = Scripted()
    await prepare_oci_workload_container(
        ctrl, "vm-ca", CapsemSandboxConfig(build={"context": str(ctx)}).to_container_spec()
    )
    assert any("certutil" in c for c in ctrl.commands)


async def test_buildkit_required_and_sample_init_immutability(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    log_path = _install_fake_docker(tmp_path, monkeypatch)
    cache_dir, ctx = tmp_path / "cache", tmp_path / "app"
    ctx.mkdir()
    df = ctx / "Dockerfile"
    df.write_text("FROM alpine:3.20\nRUN <<EOF\necho hello\nEOF\n", encoding="utf-8")
    _set_build_env(monkeypatch, tmp_path)

    spec = {"context": str(ctx), "dockerfile": str(df)}
    ib_mod.run_host_build(spec, cache_dir=cache_dir)
    assert "<<EOF" in json.loads(log_path.read_text(encoding="utf-8"))[-1]

    for disabled_val in ("0", "false", "no", "off"):
        monkeypatch.setenv("DOCKER_BUILDKIT", disabled_val)
        with pytest.raises(RuntimeError, match="require Docker BuildKit"):
            ib_mod.run_host_build(spec, cache_dir=cache_dir)
    monkeypatch.delenv("DOCKER_BUILDKIT")

    ctrl = Scripted()
    monkeypatch.setattr(sb_mod, "SdkCapsemController", lambda: ctrl)

    async def _noop_bake(*_a: Any, **_k: Any) -> None:
        return None

    monkeypatch.setattr(sb_mod, "bake_sandbox_tools_into_controller", _noop_bake)
    shared_cfg = CapsemSandboxConfig(dockerfile=str(df))
    h0 = hash(shared_cfg)
    envs = await sb_mod.CapsemSandboxEnvironment.sample_init("t", shared_cfg, {})
    env = envs["default"]
    assert isinstance(env, sb_mod.CapsemSandboxEnvironment)
    assert (env._working_dir, env._user) == ("/", None)
    assert (shared_cfg.working_dir, shared_cfg.user, hash(shared_cfg)) == (None, None, h0)
    assert not shared_cfg.to_container_spec().working_dir_explicit
    await sb_mod.CapsemSandboxEnvironment.sample_cleanup("t", None, envs, False)

    explicit_cfg = CapsemSandboxConfig(
        dockerfile=str(df), working_dir="/explicit/dir", user="2000:2000"
    )
    envs2 = await sb_mod.CapsemSandboxEnvironment.sample_init("t2", explicit_cfg, {})
    env2 = envs2["default"]
    assert isinstance(env2, sb_mod.CapsemSandboxEnvironment)
    assert env2._working_dir == "/explicit/dir" and env2._user == "2000:2000"
    await sb_mod.CapsemSandboxEnvironment.sample_cleanup("t2", None, envs2, False)
