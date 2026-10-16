"""Hermetic unit tests for `harbor_capsem._credentials` and `CapsemEnvironment.exec()` brokering."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pytest
from capsem_integrations._controller import CommandResult
from harbor.models.task.config import EnvironmentConfig
from harbor.models.trial.paths import TrialPaths
from harbor_capsem import CapsemEnvironment
from harbor_capsem._credentials import (
    broker_env_credentials,
    coerce_bool_flag,
    should_broker_credential,
    truncate_label_value,
)


@dataclass(frozen=True)
class _FakeInjectResponse:
    credential_ref: str


class _FakeCredentialsClient:
    def __init__(self) -> None:
        self.calls: list[tuple[str, str, str]] = []

    async def inject(
        self, provider: str, value: str, *, storage: str = "file"
    ) -> _FakeInjectResponse:
        self.calls.append((provider, value, storage))
        digest = f"{len(self.calls):064x}"
        return _FakeInjectResponse(credential_ref=f"credential:blake3:{digest}")


class _FakeHypervisor:
    def __init__(self) -> None:
        self.credentials = _FakeCredentialsClient()


class _FakeBrokerController:
    def __init__(self) -> None:
        self._hypervisor = _FakeHypervisor()
        self.commands: list[tuple[str, int]] = []
        self.last_start_kwargs: dict[str, Any] = {}

    async def start_vm(
        self, *, cpu_count: int, ram_gb: int, image: str | None = None, **kwargs: Any
    ) -> str:
        del cpu_count, ram_gb, image
        self.last_start_kwargs = kwargs
        return "vm-cred-0"

    async def stop_vm(self, vm_id: str) -> None:
        del vm_id

    async def close(self) -> None:
        pass

    async def upload_to_vm(self, vm_id: str, guest_path: str, data: bytes) -> None:
        del vm_id, guest_path, data

    async def download_from_vm(
        self, vm_id: str, guest_path: str, *, max_bytes: int | None = None
    ) -> bytes:
        del vm_id, guest_path, max_bytes
        return b""

    async def exec_in_vm(self, vm_id: str, command: str, *, timeout: int = 120) -> CommandResult:
        del vm_id
        self.commands.append((command, timeout))
        if command == "pwd":
            return CommandResult(exit_code=0, stdout="/workspace\n", stderr="")
        return CommandResult(exit_code=0, stdout="ok\n", stderr="")


def _make_trial_paths(tmp_path: Path) -> TrialPaths:
    trial_dir = tmp_path / "trial"
    trial_dir.mkdir(parents=True, exist_ok=True)
    return TrialPaths(trial_dir=trial_dir)


def test_coerce_bool_and_should_broker_rules() -> None:
    assert coerce_bool_flag(None, default=True) is True
    assert coerce_bool_flag("false", default=True) is False
    assert coerce_bool_flag("TRUE", default=False) is True
    assert coerce_bool_flag(1, default=False) is True
    with pytest.raises(ValueError, match="Invalid boolean option"):
        coerce_bool_flag("maybe", default=False)

    already_ref = f"credential:blake3:{'a' * 64}"
    assert should_broker_credential(already_ref) is False
    assert should_broker_credential("   ") is False
    assert should_broker_credential("local-proxy") is True
    assert should_broker_credential("sk-live-secret") is True
    truncated = truncate_label_value("é" * 200)
    assert len(truncated.encode("utf-8")) <= 255


async def test_capsem_environment_exec_brokers_and_caches_credentials(tmp_path: Path) -> None:
    env_dir = tmp_path / "env"
    env_dir.mkdir()
    ctrl = _FakeBrokerController()
    env = CapsemEnvironment(
        environment_dir=env_dir,
        environment_name="cred-task",
        session_id="cred-task__1",
        trial_paths=_make_trial_paths(tmp_path),
        task_env_config=EnvironmentConfig(
            docker_image="ubuntu:24.04", env={"OPENAI_API_KEY": "  ya29.VertexOnOpenai  "}
        ),
        controller=ctrl,
    )
    await env.start(force_build=False)
    assert ctrl.last_start_kwargs["env"]["OPENAI_API_KEY"] == f"credential:blake3:{1:064x}"

    already_ref = f"credential:blake3:{'f' * 64}"
    await env.exec(
        "claude --version",
        env={
            "ANTHROPIC_API_KEY": "sk-ant-real-key",
            "OPENAI_API_KEY": already_ref,
            "GEMINI_API_KEY": "AIzaRealGeminiKey",
            "GOOGLE_OAUTH_ACCESS_TOKEN": "ya29.RealVertexAccessToken",
            "AIDER_API_KEY": f"openai=sk-aider-openai,anthropic={already_ref}",
            "OPENAI_API_BASE": "http://127.0.0.1:11434/v1",
        },
    )
    calls = ctrl._hypervisor.credentials.calls
    assert calls == [
        ("openai", "ya29.VertexOnOpenai", "memory"),
        ("anthropic", "sk-ant-real-key", "memory"),
        ("google", "AIzaRealGeminiKey", "memory"),
        ("google", "ya29.RealVertexAccessToken", "memory"),
        ("openai", "sk-aider-openai", "memory"),
    ]
    cmd_str = ctrl.commands[-1][0]
    assert "sk-ant-real-key" not in cmd_str and "sk-aider-openai" not in cmd_str
    assert "AIzaRealGeminiKey" not in cmd_str and "ya29.RealVertexAccessToken" not in cmd_str
    assert f"ANTHROPIC_API_KEY=credential:blake3:{2:064x}" in cmd_str
    assert f"GEMINI_API_KEY=credential:blake3:{3:064x}" in cmd_str
    assert f"GOOGLE_OAUTH_ACCESS_TOKEN=credential:blake3:{4:064x}" in cmd_str
    assert f"AIDER_API_KEY=openai=credential:blake3:{5:064x},anthropic={already_ref}" in cmd_str
    assert f"OPENAI_API_KEY={already_ref}" in cmd_str
    assert "SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt" in cmd_str

    # Second exec with same key hits the per-environment cache without re-injecting.
    await env.exec("claude -p hi", env={"ANTHROPIC_API_KEY": "sk-ant-real-key"})
    assert len(ctrl._hypervisor.credentials.calls) == 5
    await env.stop(delete=True)


async def test_broker_credentials_opt_out_and_error_paths(tmp_path: Path) -> None:
    env_dir = tmp_path / "env"
    env_dir.mkdir()
    ctrl = _FakeBrokerController()
    env = CapsemEnvironment(
        environment_dir=env_dir,
        environment_name="opt-out",
        session_id="opt-out__1",
        trial_paths=_make_trial_paths(tmp_path),
        task_env_config=EnvironmentConfig(docker_image="ubuntu:24.04"),
        broker_credentials="false",
        controller=ctrl,
    )
    await env.start(force_build=False)
    await env.exec("env", env={"ANTHROPIC_API_KEY": "sk-ant-raw"})
    assert ctrl._hypervisor.credentials.calls == []
    assert "ANTHROPIC_API_KEY=sk-ant-raw" in ctrl.commands[-1][0]
    await env.stop(delete=True)

    with pytest.raises(RuntimeError, match=r"does not expose hypervisor\.credentials\.inject"):
        await broker_env_credentials(object(), {"OPENAI_API_KEY": "sk-test"}, enabled=True)
    with pytest.raises(ValueError, match="Unsupported AIDER_API_KEY provider 'cohere'"):
        await broker_env_credentials(ctrl, {"AIDER_API_KEY": "cohere=sk-secret"}, enabled=True)
    with pytest.raises(ValueError, match="Malformed AIDER_API_KEY"):
        await broker_env_credentials(ctrl, {"AIDER_API_KEY": "sk-no-equals"}, enabled=True)


async def test_environment_error_paths_output_callbacks_and_cleanup(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from harbor_capsem._trace import _validate_and_write

    with pytest.raises(ValueError, match="ATIF trajectory failed validation"):
        _validate_and_write({})

    env_dir = tmp_path / "err_env"
    env_dir.mkdir()
    ctrl = _FakeBrokerController()
    bad_mode: Any = "invalid"
    with pytest.raises(ValueError, match="Invalid execution_mode"):
        CapsemEnvironment(
            environment_dir=env_dir,
            environment_name="bad-mode",
            session_id="bad-mode__1",
            trial_paths=_make_trial_paths(tmp_path),
            task_env_config=EnvironmentConfig(docker_image="ubuntu:24.04"),
            execution_mode=bad_mode,
            controller=ctrl,
        )

    env = CapsemEnvironment(
        environment_dir=env_dir,
        environment_name="lifecycle-edge",
        session_id="lifecycle-edge__1",
        trial_paths=_make_trial_paths(tmp_path),
        task_env_config=EnvironmentConfig(docker_image="ubuntu:24.04"),
        export_trace=True,
        controller=ctrl,
    )
    await env.stop(delete=True)
    with pytest.raises(RuntimeError, match="CapsemEnvironment is not started"):
        await env.exec("pwd")

    await env.start(force_build=False)
    chunks: list[tuple[str, str]] = []

    async def _cb(text: str, stream: str) -> None:
        chunks.append((text, stream))

    monkeypatch.setattr(env, "_output_callback", lambda: _cb)
    await env.exec("echo hi")
    assert chunks == [("ok\n", "stdout")]

    closed_count = 0

    async def _track_close() -> None:
        nonlocal closed_count
        closed_count += 1

    monkeypatch.setattr(ctrl, "close", _track_close)
    env._owns_controller = True
    with pytest.raises(NotImplementedError, match=r"stop\(delete=False\) is not supported"):
        await env.stop(delete=False)
    assert closed_count == 1 and env.vm_id == ""

    async def _fail_export(**_kw: Any) -> tuple[None, None]:
        raise ValueError("simulated validator rejection")

    monkeypatch.setattr("harbor_capsem._trace.export_and_enrich_trial_trace", _fail_export)
    await env.start(force_build=False)
    env._owns_controller = True
    await env.stop(delete=True)
    assert closed_count == 2

    await env.start(force_build=False)
    env._owns_controller = True
    env._strict_trace_export = True
    with pytest.raises(ValueError, match="simulated validator rejection"):
        await env.stop(delete=True)
    assert closed_count == 3

    async def _boom(*_args: Any, **_kwargs: Any) -> None:
        raise RuntimeError("prepare failed")

    monkeypatch.setattr(
        "harbor_capsem.environment.containers.prepare_oci_workload_container", _boom
    )
    with pytest.raises(RuntimeError, match="prepare failed"):
        await env.start(force_build=False)
    assert env.vm_id == "" and closed_count == 4

    prebuilt_dir = tmp_path / "prebuilt_env"
    prebuilt_dir.mkdir()
    (prebuilt_dir / "Dockerfile").write_text("FROM ubuntu:24.04\n", encoding="utf-8")
    out1 = _resolve_prebuilt(prebuilt_dir)
    out2 = _resolve_prebuilt(prebuilt_dir)
    assert out1[1] is not None and out2[1] is not None
    assert out1[1]["context"] != out2[1]["context"]


def _resolve_prebuilt(prebuilt_dir: Path) -> tuple[str | None, dict[str, Any] | None, Any]:
    from harbor_capsem._transfer import _resolve_container_start_inputs

    out = _resolve_container_start_inputs(
        prebuilt_dir,
        execution_mode="container",
        docker_image="ubuntu:24.04",
        force_build=False,
        compose_fields={},
        allowed_host_paths=(),
        host_build=None,
    )
    assert out[1] is not None and "capsem-harbor-prebuilt" in out[1]["context"]
    return out
