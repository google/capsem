"""Hermetic host-build (`compose.yaml` `build:` / `Dockerfile`) live VM acceptance check."""

from __future__ import annotations

import os
import subprocess
import tempfile
from collections.abc import Awaitable, Callable
from pathlib import Path
from typing import Any
from unittest.mock import patch

import inspect_capsem.containers.image_build as image_build_mod
from capsem import Hypervisor
from inspect_capsem import CapsemSandboxConfig, CapsemSandboxEnvironment, HostBuildGrant
from inspect_capsem._tools import (
    INSPECT_SANDBOX_TOOLS_GUEST_PATH,
    INSPECT_SANDBOX_TOOLS_ONEDIR_PATH,
)

try:
    import tests.host_build_fixture as _hb_fixture
except ImportError:
    import importlib.util as _importlib_util

    _fixture_path = Path(__file__).resolve().with_name("host_build_fixture.py")
    _fixture_spec = _importlib_util.spec_from_file_location(
        "_capsem_host_build_fixture", _fixture_path
    )
    assert _fixture_spec is not None and _fixture_spec.loader is not None
    _hb_fixture = _importlib_util.module_from_spec(_fixture_spec)
    _fixture_spec.loader.exec_module(_hb_fixture)

_ENV_KEYS = (
    "CAPSEM_INSPECT_HOST_BUILD",
    "CAPSEM_INSPECT_ALLOWED_HOST_PATHS",
    "CAPSEM_INSPECT_BUILD_NETWORK",
    "CAPSEM_INSPECT_BUILD_CACHE_DIR",
    "CAPSEM_INSPECT_BUILD_REGISTRY_PORT",
    "CAPSEM_INSPECT_BUILD_CA_PEM_FILE",
)


async def _expect_value_error(task_name: str, cfg: CapsemSandboxConfig, expected: str) -> None:
    try:
        await CapsemSandboxEnvironment.sample_init(task_name=task_name, config=cfg, metadata={})
    except ValueError as exc:
        assert expected in str(exc), f"expected {expected!r} in {exc!r}"
    else:
        raise AssertionError(f"expected ValueError({expected!r}) for {task_name}")


async def _cleanup_sample(task: str, envs: dict[str, Any], hyp: Hypervisor, vm_id: str) -> None:
    await CapsemSandboxEnvironment.sample_cleanup(task, None, envs, False)
    await CapsemSandboxEnvironment.task_cleanup(task, None, True)
    remaining_ids = {m.id for m in (await hyp.list()).sandboxes}
    assert vm_id not in remaining_ids, f"{task} sample_cleanup leaked VM {vm_id}"


async def verify_host_build_workload_mode(
    hyp: Hypervisor,
    home_dir: Path,
    verify_session_ledger: Callable[..., Awaitable[None]],
    run_self_check: Callable[..., Awaitable[dict[str, bool | str]]] | None = None,
) -> None:
    """Verify Compose `build:` + `Dockerfile` host build, refusals, cache reuse, and live VM."""
    _hb_fixture.require_docker()
    base_tag = f"capsem-gate-base:{os.getpid()}"
    _hb_fixture.load_base_image_into_docker(base_tag)
    settings_path = home_dir / "settings.toml"
    prev_settings = settings_path.read_text(encoding="utf-8") if settings_path.exists() else None
    saved_env = {key: os.environ.get(key) for key in _ENV_KEYS}

    orig_build = image_build_mod.run_host_build
    built_digests: list[str] = []

    def _counting_build(*args: Any, **kwargs: Any) -> str:
        digest = orig_build(*args, **kwargs)
        built_digests.append(digest)
        return digest

    with (
        tempfile.TemporaryDirectory(prefix="capsem-host-build-gate-") as tmp,
        patch.object(image_build_mod, "run_host_build", _counting_build),
    ):
        root = Path(tmp)
        compose_file, project, active_escape = _hb_fixture.write_dockerfile_and_context(
            root, base_tag
        )
        ca_file = root / "corp-ca.pem"
        ca_key = root / "corp-ca.key"
        cache_dir = root / "oci-cache"
        subprocess.run(
            [
                "openssl",
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=CapsemTestBuildCA",
                "-keyout",
                str(ca_key),
                "-out",
                str(ca_file),
            ],
            check=True,
            capture_output=True,
            timeout=15,
        )
        reg_port = _hb_fixture.pick_registry_port()

        try:
            for key in _ENV_KEYS:
                os.environ.pop(key, None)
            grant_self = HostBuildGrant(allowed_contexts=(str(project),), network="none")
            for name, grant in (("gate_hb_off", None), ("gate_hb_self", grant_self)):
                cfg_ref = CapsemSandboxConfig(compose_file=str(compose_file), host_build=grant)
                await _expect_value_error(name, cfg_ref, "CAPSEM_INSPECT_HOST_BUILD")

            os.environ["CAPSEM_INSPECT_HOST_BUILD"] = "1"
            os.environ["CAPSEM_INSPECT_ALLOWED_HOST_PATHS"] = f"{project},{ca_file}"
            os.environ["CAPSEM_INSPECT_BUILD_NETWORK"] = "none"
            os.environ["CAPSEM_INSPECT_BUILD_CACHE_DIR"] = str(cache_dir)
            os.environ["CAPSEM_INSPECT_BUILD_REGISTRY_PORT"] = str(reg_port)
            os.environ["CAPSEM_INSPECT_BUILD_CA_PEM_FILE"] = str(ca_file)

            grant_ctx = HostBuildGrant(allowed_contexts=(str(root / "outside"),), network="none")
            grant_net = HostBuildGrant(allowed_contexts=(str(project),), network="default")
            for name, grant, expected in (
                ("gate_hb_symlink", None, "active_escape"),
                ("gate_hb_widen_ctx", grant_ctx, "CAPSEM_INSPECT_ALLOWED_HOST_PATHS"),
                ("gate_hb_widen_net", grant_net, "CAPSEM_INSPECT_BUILD_NETWORK"),
            ):
                cfg_ref = CapsemSandboxConfig(compose_file=str(compose_file), host_build=grant)
                await _expect_value_error(name, cfg_ref, expected)
                active_escape.unlink(missing_ok=True)

            repo_ref = f"127.0.0.1:{reg_port}/inspect-capsem/build"
            home_dir.mkdir(parents=True, exist_ok=True)
            _hb_fixture.grant_in_settings(home_dir, repo_ref)
            cfg = CapsemSandboxConfig(compose_file=str(compose_file), cpu_count=2, ram_gb=2)
            envs = await CapsemSandboxEnvironment.sample_init("gate_hb_live", cfg, {})
            vm_id = ""
            try:
                assert len(built_digests) == 1
                assert cfg.working_dir is None and cfg.user is None
                sb = envs["default"]
                assert isinstance(sb, CapsemSandboxEnvironment)
                vm_id = sb.vm_id
                assert sb.execution_mode == "container"
                ca_guest = await sb.read_file("/usr/local/share/ca-certificates/capsem-ca.crt")
                assert ca_guest.strip() == ca_file.read_text(encoding="utf-8").strip()
                tools_bin = f"{INSPECT_SANDBOX_TOOLS_ONEDIR_PATH}/inspect-sandbox-tools"
                probe = (
                    'pwd && id -u && id -g && echo "$APP_MODE" && cat stage1.txt && '
                    "cat app.txt && stat -c %a:%u:%g app.txt && cat kept.txt && "
                    "cat unpacked/from_tar.txt && cat ca_verified.txt && "
                    f"test -x {INSPECT_SANDBOX_TOOLS_GUEST_PATH} && "
                    f"test -x {tools_bin} && "
                    f"stat -c %a:%u:%g {INSPECT_SANDBOX_TOOLS_GUEST_PATH} && "
                    "test ! -e /opt/app/sub/ignored.txt && test ! -e /ignored.txt && "
                    "test ! -e /ignored_symlink && echo ignore-ok && "
                    "grep -q 'BEGIN CERTIFICATE' /usr/local/share/capsem/ca-bundle.crt && "
                    "grep -q 'BEGIN CERTIFICATE' /etc/ssl/certs/ca-certificates.crt && "
                    "echo ca-ok"
                )
                res = await sb.exec(["/bin/sh", "-c", probe])
                assert res.returncode == 0, f"probe failed: {res.stdout}\n{res.stderr}"
                expected_lines = (
                    "/opt/app/sub\n1000\n1000\nproduction\nstage1:from-compose-arg\n"
                    "exec-form-ok\nheredoc-run-ok\nheredoc-copy-ok\n640:1000:1000\n"
                    "kept-ok\ntar-extracted\nca-trust-ok\n700:1000:1000\nignore-ok\nca-ok"
                ).splitlines()
                assert res.stdout.strip().splitlines() == expected_lines
                await sb.write_file("roundtrip.txt", "a\n")
                append_res = await sb.exec(["/bin/sh", "-c", "echo b >> roundtrip.txt"])
                assert append_res.returncode == 0, append_res.stderr
                assert await sb.read_file("roundtrip.txt") == "a\nb\n"
                await sb.exec(["rm", "-f", "roundtrip.txt"])
                try:
                    await sb.exec(["id", "-u"], user="root")
                except PermissionError:
                    pass
                else:
                    raise AssertionError("expected PermissionError for user='root' in container")
                await verify_session_ledger(hyp, vm_id, expected_target="workload", marker="ca-ok")
                if run_self_check is not None:
                    sc_res = await run_self_check(
                        sb,
                        skip={"test_read_and_write_large_file_binary", "test_exec_as_user"},
                    )
                    failures = {k: v for k, v in sc_res.items() if v is not True}
                    assert not failures, (
                        f"Live self_check (non-root container) failures: {failures}"
                    )
            finally:
                await _cleanup_sample("gate_hb_live", envs, hyp, vm_id)

            _hb_fixture.restore_settings(settings_path, prev_settings)
            _hb_fixture.grant_in_settings(home_dir, f"{repo_ref}@sha256:{built_digests[0]}")
            envs2 = await CapsemSandboxEnvironment.sample_init("gate_hb_cached", cfg, {})
            vm2_id = ""
            try:
                assert len(built_digests) == 1, "expected cache hit without second docker build"
                sb2 = envs2["default"]
                assert isinstance(sb2, CapsemSandboxEnvironment)
                vm2_id = sb2.vm_id
                res2 = await sb2.exec(["cat", "kept.txt"])
                assert res2.returncode == 0 and res2.stdout.strip() == "kept-ok"
            finally:
                await _cleanup_sample("gate_hb_cached", envs2, hyp, vm2_id)
            print("INSPECT_CAPSEM_HOST_BUILD_ACCEPTANCE_OK")
        finally:
            _hb_fixture.restore_settings(settings_path, prev_settings)
            for key, val in saved_env.items():
                if val is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = val
            subprocess.run(["docker", "rmi", "-f", base_tag], check=False, capture_output=True)
