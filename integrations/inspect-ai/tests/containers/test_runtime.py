"""Container runtime launch, CA injection, healthcheck, and Dockerfile staging tests."""

from __future__ import annotations

import asyncio
import logging
import shlex
from pathlib import Path
from typing import Any, cast

import inspect_capsem.sandbox as sb_mod
import pytest
from inspect_capsem import CapsemSandboxConfig, CapsemSandboxEnvironment, CommandResult
from inspect_capsem.containers.dockerfile import (
    _CA_READY,
    _IMAGE_CA,
    _VM_CA,
    _ca_inject_command,
    _ca_nss_command,
)
from inspect_capsem.containers.runtime import (
    _build_docker_run_command,
    prepare_oci_workload_container,
    start_container_for_init,
)

from ..conftest import LocalFakeCapsemController


@pytest.mark.parametrize(
    ("network_mode", "expected"),
    [
        (None, "host"),
        ("bridge", "host"),
        ("default", "host"),
        ("none", "none"),
        ("host", "host"),
        ("container:db", "container:db"),
    ],
)
def test_docker_run_network_defaults_to_host(network_mode: str | None, expected: str) -> None:
    cmd = _build_docker_run_command(
        "c1", "img:1", CapsemSandboxConfig(network_mode=network_mode).to_container_spec(), []
    )
    argv = shlex.split(cmd)
    assert argv.count("--network") == 1
    assert argv[argv.index("--network") + 1] == expected


class _FakeVM:
    """Records VM commands; the CA probe answers `has_ca`, the system CA
    injection exits `ca_rc` and the NSS step exits `nss_rc`."""

    def __init__(self, *, has_ca: bool, ca_rc: int = 0, nss_rc: int = 0) -> None:
        self.has_ca, self.ca_rc, self.nss_rc = has_ca, ca_rc, nss_rc
        self.commands: list[str] = []
        self.timeouts: list[int] = []

    async def exec_in_vm(self, vm_id: str, command: str, *, timeout: int = 120) -> CommandResult:
        del vm_id
        self.commands.append(command)
        self.timeouts.append(timeout)
        if command.startswith(f"[ -f {_VM_CA} ]"):
            return CommandResult(0, f"{_CA_READY}\n" if self.has_ca else "", "")
        if command.startswith("docker run -d"):
            return CommandResult(0, "cid-1\n", "")
        if command.startswith("docker exec -i -u 0 cid-1"):
            return CommandResult(self.ca_rc, "", "boom" if self.ca_rc else "")
        if command in (_ca_nss_command("cid-1"), _ca_nss_command("cid-1", user="pwuser")):
            return CommandResult(self.nss_rc, "", "nss-boom" if self.nss_rc else "")
        return CommandResult(0, "", "")


def _start(vm: _FakeVM, cfg: CapsemSandboxConfig) -> str:
    return asyncio.run(start_container_for_init(cast(Any, vm), "vm-1", cfg.to_container_spec()))


def test_container_start_installs_capsem_ca() -> None:
    vm = _FakeVM(has_ca=True)
    cfg = CapsemSandboxConfig(
        image="img:1", environment={"SSL_CERT_FILE": "/mine.pem"}, build_timeout=1800
    )
    assert _start(vm, cfg) == "cid-1"
    run_idx = next(i for i, c in enumerate(vm.commands) if c.startswith("docker run -d"))
    run = vm.commands[run_idx]
    assert vm.timeouts[run_idx] == 1800
    argv = shlex.split(run)
    envs = [argv[i + 1] for i, a in enumerate(argv) if a == "-e"]
    assert f"NODE_EXTRA_CA_CERTS={_IMAGE_CA}" in envs
    # The service's own value wins: docker keeps the last -e for a name.
    ssl_envs = [e for e in envs if e.startswith("SSL_CERT_FILE=")]
    assert ssl_envs[-1] == "SSL_CERT_FILE=/mine.pem"
    inject = vm.commands[run_idx + 1]
    assert inject == _ca_inject_command("cid-1")
    assert f"< {_VM_CA}" in inject and "update-ca-certificates" in inject
    nss = vm.commands[run_idx + 2]
    assert nss == _ca_nss_command("cid-1")
    assert nss.startswith("docker exec cid-1 sh -c ")
    # POSIX sh only: images without bash (alpine, busybox) must work.
    assert "bash" not in inject and "bash" not in nss

    # When cfg.user is configured, _ca_nss_command runs as that user so $HOME/.pki/nssdb matches.
    vm_user = _FakeVM(has_ca=True)
    assert _start(vm_user, CapsemSandboxConfig(image="img:1", user="pwuser")) == "cid-1"
    assert _ca_nss_command("cid-1", user="pwuser") in vm_user.commands
    assert _ca_nss_command("cid-1", user="pwuser").startswith("docker exec -u pwuser cid-1 sh -c ")


def test_container_start_without_vm_ca_skips_injection() -> None:
    vm = _FakeVM(has_ca=False)
    _start(vm, CapsemSandboxConfig(image="img:1"))
    run = next(c for c in vm.commands if c.startswith("docker run -d"))
    assert "SSL_CERT_FILE" not in run
    assert not any(c.startswith("docker exec") for c in vm.commands)


def test_container_start_fails_when_ca_injection_fails() -> None:
    with pytest.raises(RuntimeError, match=r"Capsem CA.*boom"):
        _start(_FakeVM(has_ca=True, ca_rc=1), CapsemSandboxConfig(image="img:1"))


def test_container_start_nss_failure_only_warns(caplog: pytest.LogCaptureFixture) -> None:
    vm = _FakeVM(has_ca=True, nss_rc=1)
    with caplog.at_level(logging.WARNING, logger="inspect_capsem.containers.runtime"):
        assert _start(vm, CapsemSandboxConfig(image="img:1")) == "cid-1"
    assert "nss-boom" in caplog.text


@pytest.mark.asyncio
async def test_failed_or_missing_dockerfile_build_raises_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Failed or missing Dockerfile build raises instead of falling back."""
    controller = LocalFakeCapsemController(tmp_path)
    monkeypatch.setattr(sb_mod, "SdkCapsemController", lambda: controller)

    with pytest.raises(FileNotFoundError, match="Dockerfile not found"):
        await CapsemSandboxEnvironment.sample_init(
            task_name="df_missing",
            config=CapsemSandboxConfig(
                execution_mode="container",
                dockerfile=str(tmp_path / "nonexistent.Dockerfile"),
            ),
            metadata={},
        )
    assert len(controller.stopped_vms) == 1

    df = tmp_path / "Dockerfile"
    df.write_text("FROM scratch\nRUN false\n", encoding="utf-8")
    controller.fail_docker_build = True
    with pytest.raises(RuntimeError, match="Failed to build Dockerfile"):
        await CapsemSandboxEnvironment.sample_init(
            task_name="df_fail",
            config=CapsemSandboxConfig(execution_mode="container", dockerfile=str(df)),
            metadata={},
        )
    assert len(controller.stopped_vms) == 2


@pytest.mark.asyncio
async def test_dockerfile_build_stages_full_context_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Full build context directory (including COPY targets and nested files) is staged into VM."""
    controller = LocalFakeCapsemController(tmp_path)
    monkeypatch.setattr(sb_mod, "SdkCapsemController", lambda: controller)

    ctx_dir = tmp_path / "ctx"
    (ctx_dir / "container").mkdir(parents=True)
    (ctx_dir / "subdir").mkdir(parents=True)
    df = ctx_dir / "container" / "Custom.Dockerfile"
    df.write_text(
        "FROM ubuntu:24.04\nCOPY helper.py /app/helper.py\nCOPY subdir/data.txt /app/data.txt\n",
        encoding="utf-8",
    )
    (ctx_dir / "helper.py").write_text("print('hi')\n", encoding="utf-8")
    (ctx_dir / "subdir" / "data.txt").write_text("payload-42\n", encoding="utf-8")

    compose_path = tmp_path / "compose.yaml"
    compose_path.write_text(
        "services:\n"
        "  default:\n"
        "    build:\n"
        "      context: ./ctx\n"
        "      dockerfile: container/Custom.Dockerfile\n",
        encoding="utf-8",
    )

    envs = await CapsemSandboxEnvironment.sample_init(
        task_name="df_ctx",
        config=CapsemSandboxConfig(compose_file=str(compose_path)),
        metadata={},
    )
    try:
        env = envs["default"]
        assert isinstance(env, CapsemSandboxEnvironment)
        vm_root = controller.vms[env.vm_id]
        staged_ctx = vm_root / "tmp" / "capsem-build"
        assert (staged_ctx / "container" / "Custom.Dockerfile").is_file()
        assert (staged_ctx / "helper.py").read_text(encoding="utf-8") == "print('hi')\n"
        assert (staged_ctx / "subdir" / "data.txt").read_text(encoding="utf-8") == "payload-42\n"
    finally:
        await CapsemSandboxEnvironment.sample_cleanup(
            task_name="df_ctx", config=None, environments=envs, interrupted=False
        )


@pytest.mark.asyncio
async def test_start_container_for_init_removes_orphan_on_ca_or_healthcheck_failure() -> None:
    """If CA injection or healthcheck fails after docker run -d, the container is removed."""
    from inspect_capsem.containers.runtime import start_container_for_init

    class FailingPostRunController:
        def __init__(self, fail_step: str) -> None:
            self.fail_step = fail_step
            self.commands: list[str] = []

        async def exec_in_vm(
            self, vm_id: str, command: str, *, timeout: int = 120
        ) -> CommandResult:
            del vm_id, timeout
            self.commands.append(command)
            if "capsem-ca.crt" in command and "CAPSEM_CA_READY" in command:
                return CommandResult(exit_code=0, stdout="CAPSEM_CA_READY\n", stderr="")
            if command.startswith("docker run -d"):
                return CommandResult(exit_code=0, stdout="cid-orphan-1\n", stderr="")
            if self.fail_step == "ca" and "update-ca-certificates" in command:
                return CommandResult(exit_code=1, stdout="", stderr="ca install failed")
            if self.fail_step == "hc" and "check_hc" in command:
                return CommandResult(exit_code=1, stdout="", stderr="hc failed")
            return CommandResult(exit_code=0, stdout="", stderr="")

    ctrl_ca = FailingPostRunController("ca")
    with pytest.raises(RuntimeError, match="Failed installing the Capsem CA"):
        await start_container_for_init(
            cast(Any, ctrl_ca),
            "vm-1",
            CapsemSandboxConfig(
                execution_mode="container", image="ubuntu:24.04"
            ).to_container_spec(),
        )
    assert ctrl_ca.commands[-1] == "docker rm -f cid-orphan-1"

    ctrl_hc = FailingPostRunController("hc")
    with pytest.raises(RuntimeError, match="failed healthcheck"):
        await start_container_for_init(
            cast(Any, ctrl_hc),
            "vm-1",
            CapsemSandboxConfig(
                execution_mode="container",
                image="ubuntu:24.04",
                healthcheck={"test": ["CMD-SHELL", "check_hc"], "retries": 1, "interval": "1s"},
            ).to_container_spec(),
        )
    assert ctrl_hc.commands[-1] == "docker rm -f cid-orphan-1"


@pytest.mark.asyncio
async def test_launch_py_sed_pattern_matches_guest_artifact() -> None:
    """The sed pattern in prepare_oci_workload_container matches guest/artifacts/container/launch.py."""
    import re
    import subprocess

    repo_root = Path(__file__).resolve().parents[4]
    launch_py = repo_root / "guest" / "artifacts" / "container" / "launch.py"
    assert launch_py.is_file(), f"Missing guest artifact: {launch_py}"
    original = launch_py.read_text(encoding="utf-8")
    target_line = 'process["args"] = (metadata.get("Entrypoint") or []) + options["args"]'
    assert target_line in original

    captured: list[str] = []

    class CaptureController:
        async def exec_in_vm(
            self, vm_id: str, command: str, *, timeout: int = 120
        ) -> CommandResult:
            del vm_id, timeout
            captured.append(command)
            return CommandResult(exit_code=0, stdout="", stderr="")

    await prepare_oci_workload_container(
        cast(Any, CaptureController()),
        "vm-1",
        CapsemSandboxConfig(execution_mode="container", image="alpine:3.19").to_container_spec(),
    )
    match = re.search(r"sed -i ('[^']+') /root/\.capsem-image/launch\.py", captured[0])
    assert match is not None
    sed_expr = shlex.split(match.group(1))[0]

    proc = subprocess.run(
        ["sed", sed_expr, str(launch_py)],
        capture_output=True,
        text=True,
        check=True,
    )
    assert target_line not in proc.stdout
    assert '    process["args"] = options["args"]\n' in proc.stdout
    compile(proc.stdout, str(launch_py), "exec")
