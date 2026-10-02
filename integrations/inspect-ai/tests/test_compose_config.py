"""Inspect-facing Compose config coercion and resolution tests."""

from __future__ import annotations

from pathlib import Path
from typing import Any, cast

import inspect_capsem._compose as compose_mod
import inspect_capsem.sandbox as sb_mod
import pytest
from inspect_capsem import CapsemSandboxConfig, CapsemSandboxEnvironment

from .conftest import LocalFakeCapsemController


def test_coerce_config_variants(tmp_path: Path) -> None:
    coerce = compose_mod.coerce_config
    compose = tmp_path / "compose.yaml"
    compose.write_text("services:\n  default:\n    image: ubuntu:24.04\n    working_dir: /src\n")
    cfg = coerce(CapsemSandboxConfig(compose_file=str(compose)))
    assert (cfg.image, cfg.working_dir, cfg.execution_mode) == ("ubuntu:24.04", "/src", "container")
    plain = CapsemSandboxConfig(image="x")
    assert coerce(plain) is plain
    assert coerce(CapsemSandboxConfig(compose_file="")).compose_file == ""
    assert coerce("Dockerfile").dockerfile == "Dockerfile"
    assert coerce(str(compose)).image == "ubuntu:24.04"
    with pytest.raises(FileNotFoundError, match="Compose file not found"):
        coerce(str(tmp_path / "absent.yml"))
    assert coerce(str(tmp_path / "absent.yml"), resolve_compose=False).compose_file == str(
        tmp_path / "absent.yml"
    )
    deserialized = CapsemSandboxEnvironment.config_deserialize(
        {"compose_file": str(tmp_path / "absent.yml")}
    )
    assert deserialized.compose_file == str(tmp_path / "absent.yml")
    assert deserialized.execution_mode == "container"
    dumped_missing = CapsemSandboxConfig(compose_file=str(tmp_path / "moved.yaml")).model_dump(
        mode="json"
    )
    assert CapsemSandboxEnvironment.config_deserialize(dumped_missing).compose_file == str(
        tmp_path / "moved.yaml"
    )
    assert coerce("python:3.12-slim").image == "python:3.12-slim"
    assert coerce(None) == CapsemSandboxConfig()
    with pytest.raises(FileNotFoundError):
        compose_mod.resolve_compose_file(
            CapsemSandboxConfig(compose_file=str(tmp_path / "none.yaml"))
        )
    empty_compose = tmp_path / "empty.yaml"
    empty_compose.write_text("services: {}\n")
    empty_cfg = CapsemSandboxConfig(compose_file=str(empty_compose))
    with pytest.raises(ValueError, match="non-empty 'services' mapping"):
        compose_mod.resolve_compose_file(empty_cfg)
    no_services = tmp_path / "capsem.yaml"
    no_services.write_text("execution_mode: vm\n")
    with pytest.raises(ValueError, match="non-empty 'services' mapping"):
        coerce(str(no_services))
    assert "capsem.yaml" not in CapsemSandboxEnvironment.config_files()
    assert "capsem.yml" not in CapsemSandboxEnvironment.config_files()


def test_compose_file_config_applies_service_fields(tmp_path: Path) -> None:
    """Compose file parses service image, working_dir, env, command, volumes, ports, limits."""
    host_dir = tmp_path / "mounted_dir"
    host_dir.mkdir()
    (host_dir / "file.txt").write_text("hello", encoding="utf-8")
    compose_path = tmp_path / "compose.yaml"
    compose_path.write_text(
        "services:\n"
        "  default:\n"
        "    image: ubuntu:24.04\n"
        "    working_dir: /app\n"
        "    environment:\n"
        "      FOO: bar\n"
        "      NUM: 42\n"
        "    command: ['python3', '-m', 'http.server', '8080']\n"
        "    entrypoint: ['/bin/sh', '-c']\n"
        "    volumes:\n"
        "      - ./mounted_dir:/mnt/data:ro\n"
        "      - named_vol:/var/lib/data\n"
        "    ports:\n"
        "      - '8080:8080'\n"
        "    expose:\n"
        "      - '9090'\n"
        "    init: true\n"
        "    mem_limit: 512m\n"
        "    network_mode: bridge\n"
        "    user: '1000:1000'\n"
        "    healthcheck:\n"
        "      test: ['CMD-SHELL', 'true']\n"
        "      interval: 1s\n"
        "      retries: 2\n",
        encoding="utf-8",
    )
    import inspect_capsem._compose as compose_mod
    from inspect_ai.util import ComposeConfig, ComposeService
    from pydantic import ValidationError

    cfg = compose_mod.coerce_config(str(compose_path))
    assert cfg.execution_mode == "container"
    assert cfg.image == "ubuntu:24.04"
    assert cfg.working_dir == "/app"
    assert cfg.environment == {"FOO": "bar", "NUM": "42"}
    assert cfg.command == ("python3", "-m", "http.server", "8080")
    assert cfg.entrypoint == ("/bin/sh", "-c")
    assert cfg.volumes == (f"{host_dir.resolve()}:/mnt/data:ro", "named_vol:/var/lib/data")
    assert cfg.ports == ("8080:8080",)
    assert cfg.expose == ("9090",)
    assert cfg.init is True
    assert cfg.mem_limit == "512m"
    assert cfg.network_mode == "bridge"
    assert cfg.user == "1000:1000"
    assert cfg.healthcheck == {"test": ["CMD-SHELL", "true"], "interval": "1s", "retries": 2}

    # x-inspect_k8s_sandbox.allow_domains (service- or top-level) raises NotImplementedError.
    k8s_svc_path = tmp_path / "compose-k8s-svc.yaml"
    k8s_svc_path.write_text(
        "services:\n"
        "  default:\n"
        "    image: ubuntu:24.04\n"
        "    x-inspect_k8s_sandbox:\n"
        "      allow_domains:\n"
        "        - pypi.org\n",
        encoding="utf-8",
    )
    with pytest.raises(NotImplementedError, match="allow_domains"):
        compose_mod.coerce_config(str(k8s_svc_path))

    k8s_top_path = tmp_path / "compose-k8s-top.yaml"
    k8s_top_path.write_text(
        "x-inspect_k8s_sandbox:\n"
        "  allow_domains:\n"
        "    - pypi.org\n"
        "services:\n"
        "  default:\n"
        "    image: ubuntu:24.04\n",
        encoding="utf-8",
    )
    with pytest.raises(NotImplementedError, match="allow_domains"):
        compose_mod.coerce_config(str(k8s_top_path))

    # Unsupported knobs on CapsemSandboxConfig / coerce_config(dict) raise NotImplementedError or ValidationError.
    with pytest.raises(NotImplementedError, match="allow_domains"):
        compose_mod.coerce_config({"allow_domains": ["pypi.org"]})
    with pytest.raises(NotImplementedError, match="host_workspace_dir"):
        compose_mod.coerce_config({"host_workspace_dir": "/tmp/ws"})
    with pytest.raises(ValidationError):
        compose_mod.coerce_config({"pool_endpoint": "http://127.0.0.1:9999"})
    with pytest.raises(TypeError, match="Unsupported Capsem sandbox config type"):
        compose_mod.coerce_config(cast(Any, object()))

    # inspect_ai ComposeConfig single-service coerces properly; multi-service raises ValueError.
    cc_single = ComposeConfig(
        services={
            "default": ComposeService(image="python:3.12-slim", working_dir="/srv", user="1000")
        }
    )
    coerced_cc = compose_mod.coerce_config(cc_single)
    assert coerced_cc.execution_mode == "container"
    assert coerced_cc.image == "python:3.12-slim"
    assert coerced_cc.working_dir == "/srv"
    assert coerced_cc.user == "1000"

    cc_multi = ComposeConfig(
        services={
            "web": ComposeService(image="nginx:alpine", working_dir="/var/www"),
            "db": ComposeService(image="redis:7", user="999"),
        }
    )
    with pytest.raises(ValueError, match="Multi-service"):
        compose_mod.coerce_config(cc_multi)

    # String Containerfile and .containerfile paths coerce to dockerfile container configs.
    cf_cfg = compose_mod.coerce_config("path/to/Containerfile")
    assert cf_cfg.execution_mode == "container"
    assert cf_cfg.dockerfile == "path/to/Containerfile"
    assert "image" not in cf_cfg.model_fields_set
    ext_cf_cfg = compose_mod.coerce_config("path/to/dev.containerfile")
    assert ext_cf_cfg.execution_mode == "container"
    assert ext_cf_cfg.dockerfile == "path/to/dev.containerfile"
    assert "image" not in ext_cf_cfg.model_fields_set


@pytest.mark.asyncio
async def test_multi_service_compose_rejected_in_sample_init(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Multi-service compose.yaml and ComposeConfig fail clearly with ValueError before starting a VM."""
    from inspect_ai.util import ComposeConfig, ComposeService

    controller = LocalFakeCapsemController(tmp_path)
    monkeypatch.setattr(sb_mod, "SdkCapsemController", lambda: controller)

    compose_path = tmp_path / "compose.yaml"
    compose_path.write_text(
        "services:\n  victim:\n    image: victim:latest\n  kali:\n    image: kali:latest\n",
        encoding="utf-8",
    )

    with pytest.raises(ValueError, match="Multi-service"):
        await CapsemSandboxEnvironment.sample_init(
            task_name="cybench_multi",
            config=CapsemSandboxConfig(compose_file=str(compose_path)),
            metadata={},
        )
    assert controller.started_vms == []

    with pytest.raises(ValueError, match="Multi-service"):
        await CapsemSandboxEnvironment.sample_init(
            task_name="compose_model_multi",
            config=ComposeConfig(
                services={
                    "app": ComposeService(image="python:3.12-slim"),
                    "redis": ComposeService(image="redis:7"),
                }
            ),
            metadata={},
        )
    assert controller.started_vms == []
