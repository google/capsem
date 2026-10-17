"""Inspect-facing Compose config coercion and resolution tests."""

from __future__ import annotations

from pathlib import Path

import inspect_capsem._compose as compose_mod
import inspect_capsem.sandbox as sb_mod
import pytest
from inspect_ai.util import ComposeConfig, ComposeService
from inspect_capsem import CapsemSandboxConfig, CapsemSandboxEnvironment
from inspect_capsem.containers import HostBuildGrant

from .helpers import LocalFakeCapsemController


def test_coerce_config_variants(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.chdir(tmp_path)
    monkeypatch.delenv("CAPSEM_INSPECT_ALLOWED_HOST_PATHS", raising=False)
    monkeypatch.delenv("CAPSEM_INSPECT_HOST_BUILD", raising=False)
    coerce = compose_mod.coerce_config
    compose = tmp_path / "compose.yaml"
    compose.write_text("services:\n  default:\n    image: ubuntu:24.04\n    working_dir: /src\n")
    cfg = coerce(CapsemSandboxConfig(compose_file=str(compose)))
    assert (cfg.image, cfg.working_dir, cfg.execution_mode) == ("ubuntu:24.04", "/src", "container")
    plain = CapsemSandboxConfig(image="x")
    assert coerce(plain) is plain
    assert coerce(CapsemSandboxConfig(compose_file="")).compose_file == ""
    with pytest.raises(ValueError, match="disabled by default"):
        coerce("Dockerfile")
    monkeypatch.setenv("CAPSEM_INSPECT_HOST_BUILD", "1")
    with pytest.raises(ValueError, match="allowed_host_paths"):
        coerce("Dockerfile")
    monkeypatch.setenv("CAPSEM_INSPECT_ALLOWED_HOST_PATHS", str(tmp_path))
    df_coerced = coerce("Dockerfile")
    assert df_coerced.execution_mode == "container" and df_coerced.build is not None
    assert CapsemSandboxEnvironment.config_deserialize(df_coerced.model_dump()) == df_coerced
    assert CapsemSandboxConfig.model_validate_json(df_coerced.model_dump_json()) == df_coerced
    assert coerce(str(compose)).image == "ubuntu:24.04"
    absent = str(tmp_path / "absent.yml")
    with pytest.raises(FileNotFoundError, match="Compose file not found"):
        coerce(absent)
    assert coerce(absent, resolve_compose=False).compose_file == absent
    deserialized = CapsemSandboxEnvironment.config_deserialize({"compose_file": absent})
    assert (deserialized.compose_file, deserialized.execution_mode) == (absent, "container")
    moved = str(tmp_path / "moved.yaml")
    dumped = CapsemSandboxConfig(compose_file=moved).model_dump(mode="json")
    assert CapsemSandboxEnvironment.config_deserialize(dumped).compose_file == moved
    assert coerce("python:3.12-slim").image == "python:3.12-slim"
    with pytest.raises(FileNotFoundError):
        compose_mod.resolve_compose_file(CapsemSandboxConfig(compose_file=absent))
    (tmp_path / "empty.yaml").write_text("services: {}\n")
    with pytest.raises(ValueError, match="non-empty 'services' mapping"):
        compose_mod.resolve_compose_file(
            CapsemSandboxConfig(compose_file=str(tmp_path / "empty.yaml"))
        )
    (tmp_path / "capsem.yaml").write_text("execution_mode: vm\n")
    with pytest.raises(ValueError, match="non-empty 'services' mapping"):
        coerce(str(tmp_path / "capsem.yaml"))
    assert "capsem.yaml" not in CapsemSandboxEnvironment.config_files()
    assert "capsem.yml" not in CapsemSandboxEnvironment.config_files()


def test_compose_file_config_applies_service_fields(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Compose file parses service image, working_dir, env, command, volumes, limits."""
    monkeypatch.chdir(tmp_path)
    monkeypatch.delenv("CAPSEM_INSPECT_ALLOWED_HOST_PATHS", raising=False)
    monkeypatch.delenv("CAPSEM_INSPECT_HOST_BUILD", raising=False)
    host_dir = tmp_path / "mounted_dir"
    host_dir.mkdir()
    (host_dir / "file.txt").write_text("hello", encoding="utf-8")
    compose_path = tmp_path / "compose.yaml"
    compose_path.write_text(
        "services:\n  default:\n    image: ubuntu:24.04\n    working_dir: /app\n"
        "    environment:\n      FOO: bar\n      NUM: 42\n"
        "    command: ['python3', '-m', 'http.server', '8080']\n    entrypoint: ['/bin/sh', '-c']\n"
        "    volumes:\n      - ./mounted_dir:/mnt/data:ro\n    mem_limit: 512m\n    cpus: '2'\n"
        "    user: '1000:1000'\n    healthcheck:\n      test: ['CMD-SHELL', 'true']\n"
        "      interval: 1s\n      retries: 2\n"
    )
    cfg = compose_mod.coerce_config(str(compose_path))
    assert (cfg.execution_mode, cfg.image, cfg.working_dir) == ("container", "ubuntu:24.04", "/app")
    assert cfg.to_container_spec().working_dir_explicit is True
    assert cfg.environment == {"FOO": "bar", "NUM": "42"}
    assert cfg.command == ("/bin/sh", "-c", "python3", "-m", "http.server", "8080")
    assert cfg.volumes == (f"{host_dir.resolve()}:/mnt/data:ro",)
    assert (cfg.mem_limit, cfg.ram_gb, cfg.cpu_count, cfg.user) == ("512m", 1, 2, "1000:1000")
    assert cfg.healthcheck == {"test": ["CMD-SHELL", "true"], "interval": "1s", "retries": 2}

    explicit_cfg = CapsemSandboxConfig(
        compose_file=str(compose_path), image="python:3.11-slim", working_dir="/workspace"
    )
    resolved = compose_mod.resolve_compose_file(explicit_cfg)
    assert (resolved.image, resolved.working_dir) == ("python:3.11-slim", "/workspace")

    for k8s_yaml in (
        "services:\n  default:\n    image: u\n"
        "    x-inspect_k8s_sandbox:\n      allow_domains: [a]\n",
        "x-inspect_k8s_sandbox:\n  allow_domains: [a]\nservices:\n  default:\n    image: u\n",
    ):
        (tmp_path / "k8s.yaml").write_text(k8s_yaml)
        with pytest.raises(NotImplementedError, match="allow_domains"):
            compose_mod.coerce_config(str(tmp_path / "k8s.yaml"))

    with pytest.raises(ValueError, match="allowed_host_paths"):
        compose_mod.coerce_config({"image": "alpine:3.20", "volumes": [f"{host_dir}:/mnt:ro"]})
    with pytest.raises(ValueError, match="allowed_host_paths"):
        compose_mod.coerce_config(
            ComposeConfig(services={"default": ComposeService(image="a", volumes=["./rel:/mnt"])})
        )

    coerced_cc = compose_mod.coerce_config(
        ComposeConfig(
            services={
                "default": ComposeService(image="python:3.12-slim", working_dir="/srv", user="1000")
            }
        )
    )
    assert (coerced_cc.execution_mode, coerced_cc.image, coerced_cc.working_dir) == (
        "container",
        "python:3.12-slim",
        "/srv",
    )
    assert coerced_cc.to_container_spec().working_dir_explicit is True

    for forbidden_direct in (
        {"build": {"context": "/etc", "dockerfile": "/etc/passwd"}},
        {"build": str(Path.home())},
        {"dockerfile": "/etc/hostname"},
        {"image": "alpine:3.20", "registry_ca_pem": "-----BEGIN CERTIFICATE-----"},
    ):
        with pytest.raises(ValueError):
            compose_mod.coerce_config(forbidden_direct)

    df_file = tmp_path / "Dockerfile"
    df_file.write_text("FROM alpine:3.20\nWORKDIR /app/work\nUSER 1002:1002\n", encoding="utf-8")
    grant = HostBuildGrant(allowed_contexts=(str(tmp_path),))
    with pytest.raises(ValueError, match="CAPSEM_INSPECT_HOST_BUILD"):
        compose_mod.coerce_config({"dockerfile": str(df_file), "host_build": grant})

    monkeypatch.setenv("CAPSEM_INSPECT_HOST_BUILD", "1")
    monkeypatch.setenv("CAPSEM_INSPECT_ALLOWED_HOST_PATHS", str(tmp_path))
    direct_df = compose_mod.coerce_config({"dockerfile": str(df_file), "host_build": grant})
    assert direct_df.build is not None and direct_df.execution_mode == "container"
    assert CapsemSandboxEnvironment.config_deserialize(direct_df.model_dump()) == direct_df
    assert CapsemSandboxConfig.model_validate_json(direct_df.model_dump_json()) == direct_df
    cc_build = compose_mod.coerce_config(
        ComposeConfig(services={"default": ComposeService(build=str(tmp_path))})
    )
    assert cc_build.build is not None and cc_build.execution_mode == "container"
    assert CapsemSandboxEnvironment.config_deserialize(cc_build.model_dump()) == cc_build
    assert CapsemSandboxConfig.model_validate_json(cc_build.model_dump_json()) == cc_build

    expected_roots = (str(tmp_path.resolve()),)
    for df_str in ("Dockerfile", "Containerfile", "path/to/Custom.Dockerfile", "dev.containerfile"):
        coerced_df = compose_mod.coerce_config(df_str)
        assert coerced_df.execution_mode == "container" and coerced_df.build is not None
        assert coerced_df.to_container_spec().allowed_host_paths == expected_roots
    assert compose_mod.coerce_config({"dockerfile": str(df_file)}).build is not None
    assert compose_mod.coerce_config({"build": str(tmp_path)}).build is not None
    for bad_dict_key in ("build_context", "build_args", "build_target", "dockerfile_inline"):
        with pytest.raises(ValueError, match=bad_dict_key):
            compose_mod.coerce_config({bad_dict_key: "x"})


async def test_multi_service_compose_rejected_in_sample_init(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Multi-service compose.yaml and ComposeConfig fail with ValueError before starting a VM."""
    controller = LocalFakeCapsemController(tmp_path)
    monkeypatch.setattr(sb_mod, "SdkCapsemController", lambda: controller)
    compose_path = tmp_path / "compose.yaml"
    compose_path.write_text("services:\n  v:\n    image: v:1\n  k:\n    image: k:1\n")
    for bad_cfg in (
        CapsemSandboxConfig(compose_file=str(compose_path)),
        ComposeConfig(services={"a": ComposeService(image="a"), "b": ComposeService(image="b")}),
    ):
        with pytest.raises(ValueError, match="Multi-service"):
            await CapsemSandboxEnvironment.sample_init(
                task_name="multi", config=bad_cfg, metadata={}
            )
    assert controller.started_vms == []
