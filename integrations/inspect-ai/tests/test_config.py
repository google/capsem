"""Package imports, entry-point registration, and `CapsemSandboxConfig` tests."""

from __future__ import annotations

import hashlib
import io
import json
import os
import tarfile
import textwrap
import tomllib
from pathlib import Path
from typing import Any, cast

import inspect_capsem
import pytest
from capsem_integrations._controller import SdkCapsemController
from capsem_integrations._users import is_root_user_spec
from capsem_integrations.containers import image_build as ib_mod
from capsem_integrations.containers.compose_fields import (
    _is_host_path_allowed,
    extract_capsem_compose_fields,
)
from capsem_integrations.containers.runtime import (
    _stage_oci_bind_volumes,
    prepare_oci_workload_container,
)
from inspect_ai.util._sandbox.registry import registry_find_sandboxenv
from inspect_capsem import CapsemSandboxConfig, CapsemSandboxEnvironment
from inspect_capsem import sandbox as sb_mod
from inspect_capsem._compose import coerce_config
from inspect_capsem._exec import _format_exec_command
from pydantic import ValidationError

from .helpers import Scripted, ok


def test_package_exports_and_registry() -> None:
    assert inspect_capsem.__all__ == ["CapsemSandboxConfig", "CapsemSandboxEnvironment"]
    manifest = Path(__file__).resolve().parents[3] / "integrations/inspect-ai/pyproject.toml"
    pyproject = tomllib.loads(manifest.read_text(encoding="utf-8"))
    capsem_dep = next(d for d in pyproject["project"]["dependencies"] if d.startswith("capsem>="))
    floor = tuple(int(part) for part in capsem_dep.removeprefix("capsem>=").split("."))
    assert floor >= (0, 7, 0)
    assert registry_find_sandboxenv("capsem") is CapsemSandboxEnvironment
    assert SdkCapsemController is not None


def test_config_roundtrip_and_hash() -> None:
    default_cfg = CapsemSandboxConfig()
    assert default_cfg.cpu_count == 4 and default_cfg.ram_gb == 8
    assert CapsemSandboxEnvironment.config_deserialize(default_cfg.model_dump(mode="json")) == (
        default_cfg
    )
    cfg = CapsemSandboxConfig(
        cpu_count=2, ram_gb=4, working_dir="/tmp", environment={"FOO": "bar"}, user="developer"
    )
    assert cfg.cpu_count == 2 and hash(default_cfg) != hash(cfg)
    restored = CapsemSandboxEnvironment.config_deserialize(cfg.model_dump(mode="json"))
    assert isinstance(restored, CapsemSandboxConfig) and restored == cfg


def test_coerce_config_and_unsupported_knobs() -> None:
    assert coerce_config(None) == CapsemSandboxConfig()
    cfg = CapsemSandboxConfig(cpu_count=2)
    assert coerce_config(cfg) is cfg
    assert coerce_config({"cpu_count": 2}) == CapsemSandboxConfig(cpu_count=2)
    with pytest.raises(ValidationError, match="requires an explicit OCI image reference"):
        CapsemSandboxConfig(execution_mode="container")
    with pytest.raises(ValueError, match="requires an explicit OCI image reference"):
        CapsemSandboxConfig(compose_file="compose.yaml").to_container_spec()
    with pytest.raises(ValidationError):
        coerce_config({"pool_endpoint": "http://127.0.0.1:9999"})
    with pytest.raises(TypeError, match="Unsupported Capsem sandbox config type"):
        coerce_config(cast(Any, object()))


async def test_host_path_containment_and_operator_allowlist_plumbing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    compose_dir, sibling_dir, outside_dir = tmp_path / "a", tmp_path / "ab", tmp_path / "outside"
    for d in (compose_dir, sibling_dir, outside_dir):
        d.mkdir()
    outside_file, sibling_file = outside_dir / "secret.txt", sibling_dir / "sibling.txt"
    outside_file.write_text("secret")
    sibling_file.write_text("sibling")
    escape_dir_link, escape_file_link = compose_dir / "escape_dir", compose_dir / "escape.txt"
    escape_dir_link.symlink_to(outside_dir)
    escape_file_link.symlink_to(outside_file)

    for p in (escape_dir_link, escape_file_link, sibling_dir, sibling_file):
        assert not _is_host_path_allowed(p, (str(compose_dir),), base_dir=compose_dir)

    allow_link = tmp_path / "allow_link"
    allow_link.symlink_to(outside_dir)
    assert _is_host_path_allowed(outside_file, (str(allow_link),))
    assert not _is_host_path_allowed(tmp_path / "other.txt", (str(allow_link),))
    monkeypatch.chdir(tmp_path)
    assert _is_host_path_allowed(outside_file, ("outside",))
    assert not _is_host_path_allowed(sibling_file, ("outside",))

    monkeypatch.setenv("HOME", str(outside_dir))
    monkeypatch.delenv("CAPSEM_ALLOWED_HOST_PATHS", raising=False)
    monkeypatch.delenv("CAPSEM_INSPECT_ALLOWED_HOST_PATHS", raising=False)
    for bad_vol in (
        "./escape_dir:/mnt",
        "./escape.txt:/mnt/s.txt",
        "../ab:/mnt",
        "../outside/secret.txt:/mnt/s.txt",
        f"{outside_dir}:/mnt",
        "~/secret.txt:/mnt/s.txt",
        {"type": "bind", "source": "nonexistent_in_cwd", "target": "/mnt"},
    ):
        doc = {"services": {"app": {"image": "alpine:3.20", "volumes": [bad_vol]}}}
        with pytest.raises(ValueError, match="allowed_host_paths"):
            extract_capsem_compose_fields(doc, base_dir=compose_dir)

    vol_doc = {"services": {"app": {"image": "a", "volumes": [f"{outside_file}:/mnt/s.txt:ro"]}}}
    allow_out = (str(outside_dir),)
    with pytest.raises(ValueError, match="allowed_host_paths"):
        extract_capsem_compose_fields(vol_doc, base_dir=compose_dir, allowed_host_paths=allow_out)

    monkeypatch.setenv("CAPSEM_INSPECT_ALLOWED_HOST_PATHS", f"{outside_dir},{sibling_dir}")
    ext_ok = extract_capsem_compose_fields(
        vol_doc, base_dir=compose_dir, allowed_host_paths=allow_out
    )
    assert ext_ok["volumes"] == (f"{outside_file.resolve()}:/mnt/s.txt:ro",)

    sib_doc = {"services": {"app": {"image": "a", "volumes": [f"{sibling_file}:/mnt/sib.txt:ro"]}}}
    with pytest.raises(ValueError, match="allowed_host_paths"):
        extract_capsem_compose_fields(sib_doc, base_dir=compose_dir, allowed_host_paths=allow_out)

    allow_cmp = (str(compose_dir),)
    with pytest.raises(ValueError, match="allowed_host_paths"):
        await _stage_oci_bind_volumes(
            cast(Any, Scripted()),
            "vm-1",
            (f"{escape_file_link}:/mnt/s.txt",),
            allowed_host_paths=allow_cmp,
        )

    monkeypatch.delenv("CAPSEM_ALLOWED_HOST_PATHS", raising=False)
    monkeypatch.delenv("CAPSEM_INSPECT_ALLOWED_HOST_PATHS", raising=False)
    spec_unallowed = CapsemSandboxConfig(
        execution_mode="container",
        image="alpine:3.20",
        volumes=(f"{outside_file}:/mnt/secret.txt",),
        allowed_host_paths=allow_out,
    ).to_container_spec()
    assert spec_unallowed.allowed_host_paths == ()
    with pytest.raises(ValueError, match="allowed_host_paths"):
        await prepare_oci_workload_container(Scripted(), "vm-1", spec_unallowed)

    in_dir = compose_dir / "data"
    in_dir.mkdir()
    (in_dir / "in.txt").write_text("in")
    compose_path = compose_dir / "compose.yaml"
    compose_path.write_text(
        "services:\n  app:\n    image: alpine:3.20\n    volumes:\n      - ./data:/mnt/data:ro\n"
    )
    spec_ok = coerce_config(str(compose_path)).to_container_spec()
    await prepare_oci_workload_container(Scripted([("", ok())]), "vm-1", spec_ok)


def test_user_spec_quoting_and_mixed_groups() -> None:
    assert is_root_user_spec("0:0") and is_root_user_spec("root:root")
    assert not is_root_user_spec("0:1000") and not is_root_user_spec("root:nogroup")
    inj_cmd, _ = _format_exec_command(
        "id", effective_cwd="/", env=None, user='$(id); "pwn"', timeout=None
    )
    assert "echo 'capsem: unknown user $(id); \"pwn\"' >&2; exit 1;" in inj_cmd
    cmd_0_1000, _ = _format_exec_command(
        "id -g", effective_cwd="/", env=None, user="0:1000", timeout=None
    )
    assert "setpriv --reuid=0 --regid=1000 --clear-groups /bin/sh -c" in cmd_0_1000
    for spec, expected in (
        ("1000:users", ("_uid=1000;", "getent group users")),
        ("alice:1000", ("_uid=$(id -u alice 2>/dev/null)", "_gid=1000;")),
        ("alice:users", ("_uid=$(id -u alice 2>/dev/null)", "getent group users")),
    ):
        cmd, _ = _format_exec_command("id", effective_cwd="/", env=None, user=spec, timeout=None)
        assert all(part in cmd for part in expected)
        assert 'setpriv --reuid="$_uid" --regid="$_gid" --clear-groups /bin/sh -c' in cmd


def _install_fake_docker(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    bin_dir, tar_path = tmp_path / "bin", tmp_path / "template_oci.tar"
    bin_dir.mkdir(exist_ok=True)
    cfg_b, lay_b = b'{"architecture":"amd64","os":"linux"}', b"layer-tar-content"
    cfg_h, lay_h = hashlib.sha256(cfg_b).hexdigest(), hashlib.sha256(lay_b).hexdigest()
    m_mt, c_mt, l_mt = (
        "application/vnd.oci.image.manifest.v1+json",
        "application/vnd.oci.image.config.v1+json",
        "application/vnd.oci.image.layer.v1.tar",
    )
    m_b = json.dumps(
        {
            "schemaVersion": 2,
            "mediaType": m_mt,
            "config": {"mediaType": c_mt, "digest": f"sha256:{cfg_h}", "size": len(cfg_b)},
            "layers": [{"mediaType": l_mt, "digest": f"sha256:{lay_h}", "size": len(lay_b)}],
        },
        separators=(",", ":"),
    ).encode()
    m_h = hashlib.sha256(m_b).hexdigest()
    idx_b = json.dumps(
        {
            "schemaVersion": 2,
            "manifests": [{"mediaType": m_mt, "digest": f"sha256:{m_h}", "size": len(m_b)}],
        }
    ).encode()
    with tarfile.open(tar_path, "w") as tf:
        for name, data in (
            (f"blobs/sha256/{cfg_h}", cfg_b),
            (f"blobs/sha256/{lay_h}", lay_b),
            (f"blobs/sha256/{m_h}", m_b),
            ("index.json", idx_b),
        ):
            info = tarfile.TarInfo(name=name)
            info.size = len(data)
            tf.addfile(info, io.BytesIO(data))
    docker_bin, log_path = bin_dir / "docker", tmp_path / "docker_builds.json"
    docker_bin.write_text(
        textwrap.dedent(
            """\
            #!/usr/bin/env python3
            import json, os, shutil, sys
            from pathlib import Path
            args = sys.argv[3:] if sys.argv[1:2] == ["--config"] else sys.argv[1:]
            if args[:2] == ["context", "inspect"]:
                print("unix:///var/run/docker.sock")
            if args[:1] == ["build"]:
                df = Path(args[args.index("-f") + 1]).read_text(encoding="utf-8")
                log_p = Path(os.environ["FAKE_DOCKER_LOG"])
                prev = json.loads(log_p.read_text(encoding="utf-8")) if log_p.is_file() else []
                log_p.write_text(json.dumps([*prev, df]), encoding="utf-8")
            if args[:2] == ["image", "save"]:
                shutil.copyfile(os.environ["FAKE_DOCKER_TAR"], args[args.index("-o") + 1])
            """
        ),
        encoding="utf-8",
    )
    docker_bin.chmod(0o755)
    monkeypatch.setenv("PATH", f"{bin_dir}:{os.environ.get('PATH', '')}")
    monkeypatch.setenv("FAKE_DOCKER_LOG", str(log_path))
    monkeypatch.setenv("FAKE_DOCKER_TAR", str(tar_path))
    for k, v in (
        ("HOST_BUILD", "1"),
        ("ALLOWED_HOST_PATHS", str(tmp_path)),
        ("BUILD_CACHE_DIR", str(tmp_path / "cache")),
        ("BUILD_REGISTRY_PORT", "0"),
    ):
        monkeypatch.setenv(f"CAPSEM_INSPECT_{k}", v)
    return log_path


async def test_buildkit_required_and_sample_init_immutability(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    log_path = _install_fake_docker(tmp_path, monkeypatch)
    cache_dir, ctx = tmp_path / "cache", tmp_path / "app"
    ctx.mkdir()
    df = ctx / "Dockerfile"
    df.write_text("FROM alpine:3.20\nRUN <<EOF\necho hello\nEOF\n", encoding="utf-8")

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
