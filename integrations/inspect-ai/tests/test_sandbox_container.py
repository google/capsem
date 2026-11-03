"""OCI container mode initialization, working_dir resolution, and exec tests."""

from __future__ import annotations

import os
import shutil
import subprocess as sp
from pathlib import Path

import capsem_integrations._controller as ctrl_mod
import capsem_integrations._exec as exec_mod
import inspect_capsem.sandbox as sb_mod
import pytest
from inspect_capsem import CapsemSandboxConfig, CapsemSandboxEnvironment
from inspect_capsem._files import _chown_to_container_user_snippet

from .helpers import (
    LocalFakeCapsemController,
    Scripted,
    _host_timeout_skips,
    _run_inspect_self_check,
    env_for,
    fail,
    ok,
)


def test_oci_create_command_and_image_normalization() -> None:
    assert ctrl_mod._normalize_image_ref(None) is None
    assert ctrl_mod._normalize_image_ref("  ") is None
    assert ctrl_mod._normalize_image_ref("python:3.12-slim") == "docker://python:3.12-slim"
    assert ctrl_mod._normalize_image_ref("docker://alpine:3.19") == "docker://alpine:3.19"
    assert sb_mod._oci_create_command(CapsemSandboxConfig(image="alpine:3.19")) is None
    assert (
        sb_mod._oci_create_command(CapsemSandboxConfig(image="alpine:3.19", command="   ")) is None
    )
    assert sb_mod._oci_create_command(
        CapsemSandboxConfig(image="alpine:3.19", command="python3 -m http.server 8080")
    ) == ("python3", "-m", "http.server", "8080")
    assert sb_mod._oci_create_command(
        CapsemSandboxConfig(image="alpine:3.19", command=("sleep", "infinity"))
    ) == ("sleep", "infinity")


async def test_container_mode_uses_oci_create_and_resolves_workdir(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Container mode passes image/command to start_vm and probes WORKDIR when omitted."""
    fake = Scripted([("pwd", ok("/app/src\n"))])
    monkeypatch.setattr(sb_mod, "SdkCapsemController", lambda: fake)
    compose_path = tmp_path / "compose.yaml"
    compose_path.write_text("services:\n  default:\n    image: swe-bench/eval:1.0\n")
    envs = await CapsemSandboxEnvironment.sample_init(
        task_name="oci_default_wd", config=str(compose_path), metadata={}
    )
    try:
        sb = envs["default"]
        assert isinstance(sb, CapsemSandboxEnvironment)
        assert sb.execution_mode == "container"
        assert fake.started[-1]["image"] == "swe-bench/eval:1.0"
        assert not any(c.startswith("docker ") for c in fake.commands)
        fake.commands.clear()
        await sb.exec(["pytest", "-q"])
        assert len(fake.commands) == 1 and fake.commands[0].startswith("sh -c ")
        assert "cd /app/src && pytest -q" in fake.commands[0]
        assert exec_mod._CONTAINER_HOME_FIX in fake.commands[0]
        fake.commands.clear()
        await sb.exec(["pytest", "-q"], user="1000:1000", timeout=None)
        assert len(fake.commands) == 1 and not fake.commands[0].startswith("sh -c ")
    finally:
        await CapsemSandboxEnvironment.sample_cleanup(
            task_name="oci_default_wd", config=None, environments=envs, interrupted=False
        )


async def test_oci_container_write_file_chowns_to_nonroot_user(
    tmp_path: Path, caplog: pytest.LogCaptureFixture
) -> None:
    """OCI container write_file chowns to cfg.user or /proc/1 and warns on failure."""
    bin_dir, chown_log = tmp_path / "bin", tmp_path / "chown.log"
    bin_dir.mkdir()
    fake_chown = bin_dir / "chown"
    fake_chown.write_text(f'#!/bin/sh\necho "$@" >> "{chown_log}"\n')
    fake_chown.chmod(0o755)
    env = {**os.environ, "PATH": f"{bin_dir}:{os.environ.get('PATH', '/usr/bin:/bin')}"}

    for cfg_user, arg, expected in (
        ("developer", "imageuser", "developer: /home/dev/file.py"),
        ("", "1000:1000", "1000:1000 /home/dev/file.py"),
    ):
        snip = _chown_to_container_user_snippet("'/home/dev/file.py'", cfg_user)
        sp.run(["/bin/sh", "-c", snip, "sh", arg], env=env, check=True)
        assert chown_log.read_text().strip() == expected
        chown_log.unlink()

    snip_root = _chown_to_container_user_snippet("'/root/file.py'", "root")
    sp.run(["/bin/sh", "-c", snip_root, "sh", "1000:1000"], env=env, check=True)
    assert not chown_log.exists()

    ctrl_oci = Scripted([("", ok())])
    sb_oci = env_for(ctrl_oci, execution_mode="container", user="1000:1000")
    await sb_oci.write_file("/workspace/fix.py", "x = 1\n")
    assert all(s in "\n".join(ctrl_oci.commands) for s in ("/proc/1", "chown ", "1000:1000"))

    caplog.clear()
    ctrl_warn = Scripted([("chown", fail(1, stderr="chown: invalid group"))])
    sb_warn = env_for(ctrl_warn, execution_mode="container", user="bad:group")
    await sb_warn.write_file("/workspace/b.py", "1")
    assert "Failed to chown /workspace/b.py in container" in caplog.text


async def test_container_mode_passes_self_check_with_local_fake_controller(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """`execution_mode='container'` passes Inspect `self_check` via `LocalFakeCapsemController`."""
    controller = LocalFakeCapsemController(tmp_path, skip_bake=False)
    monkeypatch.setattr(sb_mod, "SdkCapsemController", lambda: controller)
    guest_work = tmp_path / "container_work"
    guest_work.mkdir()
    cfg = CapsemSandboxConfig(
        execution_mode="container", image="ubuntu:24.04", working_dir=str(guest_work)
    )
    envs = await CapsemSandboxEnvironment.sample_init(
        task_name="self_check_container_mode", config=cfg, metadata={}
    )
    env = envs["default"]
    assert isinstance(env, CapsemSandboxEnvironment)
    assert env.execution_mode == "container"
    try:
        skip = {
            "test_read_and_write_large_file_binary",
            "test_exec_input_large",
            "test_read_file_limit",
            "test_exec_as_user",
        } | _host_timeout_skips()
        results = await _run_inspect_self_check(env, skip=skip)
        failures = {k: v for k, v in results.items() if v is not True}
        assert failures == {}, f"Inspect container self_check failures: {failures}"
    finally:
        await CapsemSandboxEnvironment.sample_cleanup(
            task_name="self_check_container_mode", config=None, environments=envs, interrupted=False
        )


def test_format_exec_command_user_specs_at_and_not_at_target_identity(tmp_path: Path) -> None:
    """All 4 user spec forms skip su/setpriv at target identity and switch when not at target."""
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    for name, body in (
        (
            "id",
            '#!/bin/sh\nflag="$1"; target="${2:-}"\n'
            'if [ -z "$target" ]; then\n'
            '  [ "$flag" = "-u" ] && echo "${FAKE_UID:-1000}" && exit 0\n'
            '  [ "$flag" = "-g" ] && echo "${FAKE_GID:-1000}" && exit 0\n'
            '  [ "$flag" = "-un" ] && echo "appuser" && exit 0\n'
            "fi\n"
            'if [ "$target" = "appuser" ] || [ "$target" = "1000" ]; then\n'
            '  [ "$flag" = "-un" ] && echo "appuser" || echo "1000"\n'
            "  exit 0\n"
            "fi\n"
            'if [ "$target" = "otheruser" ]; then echo "2000"; exit 0; fi\n'
            "exit 1\n",
        ),
        (
            "getent",
            '#!/bin/sh\n[ "$1:$2" = "group:appgroup" ] && echo "appgroup:x:1000:" && exit 0\n'
            '[ "$1:$2" = "passwd:1000" ] && echo "appuser:x:1000:1000::/home/appuser:/bin/sh" '
            "&& exit 0\nexit 2\n",
        ),
        ("su", '#!/bin/sh\n[ "${DENY_SWITCH:-0}" = "1" ] && exit 99\nprintf "su:%s\\n" "$*"\n'),
        (
            "setpriv",
            '#!/bin/sh\n[ "${DENY_SWITCH:-0}" = "1" ] && exit 99\nprintf "setpriv:%s\\n" "$*"\n',
        ),
    ):
        p = bin_dir / name
        p.write_text(body, encoding="utf-8")
        p.chmod(0o755)

    base_env = {**os.environ, "PATH": f"{bin_dir}:{os.environ.get('PATH', '/usr/bin:/bin')}"}
    at_env = {**base_env, "FAKE_UID": "1000", "FAKE_GID": "1000", "DENY_SWITCH": "1"}
    fmt = exec_mod._format_exec_command
    for spec in ("1000", "1000:1000", "appuser", "appuser:appgroup", "1000:appgroup"):
        cmd = 'printf "%s:%s:%s" "$USER" "$LOGNAME" "$HOME"'
        script, _ = fmt(cmd, effective_cwd="/", env=None, user=spec, timeout=None)
        res = sp.run(["/bin/sh", "-c", script], env=at_env, check=True, capture_output=True)
        assert res.stdout.decode() == "appuser:appuser:/home/appuser"

    for mismatch_spec in ("2000", "2000:2000", "otheruser", "otheruser:appgroup", "1000:2000"):
        script, _ = exec_mod._format_exec_command(
            "true", effective_cwd="/", env=None, user=mismatch_spec, timeout=None
        )
        deny_res = sp.run(["/bin/sh", "-c", script], env=at_env, check=False, capture_output=True)
        assert deny_res.returncode == 126
        assert "no-new-privileges" in deny_res.stderr.decode()

    for root_spec in ("root", "0", "0:0", "root:root"):
        inner, _ = exec_mod._format_exec_command(
            "printf ok", effective_cwd="/", env=None, user=root_spec, timeout=None
        )
        wrapped = exec_mod._wrap_target_command(inner, is_container=True, user=root_spec)
        assert exec_mod._CONTAINER_ROOT_CHECK in wrapped
        deny_root = sp.run(["/bin/sh", "-c", wrapped], env=at_env, check=False, capture_output=True)
        assert deny_root.returncode == 126
        assert "cannot switch to root inside a non-root workload" in deny_root.stderr.decode()

    root_env = {**base_env, "FAKE_UID": "0", "FAKE_GID": "0", "DENY_SWITCH": "0"}
    ok_root = sp.run(
        ["/bin/sh", "-c", exec_mod._wrap_target_command("printf ok", is_container=True, user="0")],
        env=root_env,
        check=True,
        capture_output=True,
    )
    assert ok_root.stdout.decode() == "ok"

    for spec, expected_prefix in (
        ("1000", "su:-m appuser -s /bin/sh -c "),
        ("2000", "setpriv:--reuid=2000 --regid=0 --clear-groups /bin/sh -c "),
        ("1000:1000", "setpriv:--reuid=1000 --regid=1000 --clear-groups /bin/sh -c "),
        ("appuser", "su:-m appuser -s /bin/sh -c "),
        ("appuser:appgroup", "setpriv:--reuid=1000 --regid=1000 --clear-groups /bin/sh -c "),
    ):
        script, _ = fmt("true", effective_cwd="/", env=None, user=spec, timeout=None)
        res = sp.run(["/bin/sh", "-c", script], env=root_env, check=True, capture_output=True)
        assert res.stdout.decode().startswith(expected_prefix)

    for bad_spec, err_msg in (("ghost", "unknown user"), ("appuser:ghostgrp", "unknown group")):
        script, _ = fmt("true", effective_cwd="/", env=None, user=bad_spec, timeout=None)
        bad_res = sp.run(["/bin/sh", "-c", script], env=root_env, check=False, capture_output=True)
        assert bad_res.returncode == 1 and err_msg in bad_res.stderr.decode()


def test_oci_fixture_has_no_bash_split_timeout_shims_and_posix_fallbacks(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Fixture tar omits bash/split/timeout shims and timeout/split fallbacks run in /bin/sh."""
    import io
    import tarfile

    from .oci_workload_fixture import _build_rootfs_tar_bytes

    with tarfile.open(fileobj=io.BytesIO(_build_rootfs_tar_bytes(b"busybox")), mode="r") as tf:
        names = set(tf.getnames())
    assert {"bin/sh", "bin/busybox"} <= names
    assert names.isdisjoint({"bin/bash", "usr/bin/bash", "bin/split", "bin/timeout"})

    bin_dir = tmp_path / "minbin"
    bin_dir.mkdir()
    for tool in ("sh", "date", "sleep", "rm", "cat", "head", "mkdir", "mktemp", "ls"):
        for cand in (Path(f"/bin/{tool}"), Path(f"/usr/bin/{tool}")):
            if cand.exists():
                (bin_dir / tool).symlink_to(cand)
                break
    sh, min_env, fmt = str(bin_dir / "sh"), {"PATH": str(bin_dir)}, exec_mod._format_exec_command
    script, _ = fmt("sleep 5", effective_cwd=str(tmp_path), env=None, user=None, timeout=1)
    assert "mktemp -d /tmp/.capsem_to_XXXXXX" in script and "/tmp/.capsem_to_$$" not in script
    res = sp.run([sh, "-c", script], env=min_env, capture_output=True)
    assert res.returncode == 124 and exec_mod._TIMEOUT_SENTINEL.encode() in res.stderr

    (bin_dir / "mktemp").unlink()
    tok, mode_f = os.urandom(6).hex(), tmp_path / "mode"
    glob_p = f".capsem_to_{tok}_*"
    monkeypatch.setattr(exec_mod.uuid, "uuid4", lambda: type("U", (), {"hex": tok})())
    m_cmd = f"ls -ld /tmp/{glob_p} > {mode_f} && sleep 5"
    no_mkt, _ = fmt(m_cmd, effective_cwd=str(tmp_path), env=None, user=None, timeout=1)
    try:
        res2 = sp.run([sh, "-c", no_mkt], env=min_env, capture_output=True)
        assert res2.returncode == 124 and exec_mod._TIMEOUT_SENTINEL.encode() in res2.stderr
        assert mode_f.read_text().startswith("drwx------") and not any(Path("/tmp").glob(glob_p))
        col = sp.run(
            [sh, "-c", f"printf keep > /tmp/.capsem_to_{tok}_$$ && {no_mkt}"],
            env=min_env,
            capture_output=True,
        )
        [collided] = list(Path("/tmp").glob(glob_p))
        assert col.returncode == 125 and collided.read_text() == "keep"
    finally:
        for p in Path("/tmp").glob(glob_p):
            shutil.rmtree(p, ignore_errors=True) if p.is_dir() else p.unlink(missing_ok=True)


async def test_container_nonroot_user_switch_maps_to_permission_error() -> None:
    """In container mode, exit 126 with no-new-privileges raises PermissionError."""
    err = "capsem: cannot switch {} inside a non-root workload (no-new-privileges)\n"
    ctrl = Scripted(
        [
            ("cannot switch to root", fail(126, stderr=err.format("to root"))),
            ("cannot switch user", fail(126, stderr=err.format("user"))),
        ]
    )
    sb = env_for(ctrl, execution_mode="container")
    for requested_user in ("root", "0", "2000", "2000:2000", "nobody"):
        with pytest.raises(PermissionError, match="no-new-privileges"):
            await sb.exec(["id", "-u"], user=requested_user)
