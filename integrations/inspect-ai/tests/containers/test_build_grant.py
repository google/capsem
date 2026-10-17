"""Unit tests for `build_grant` (`HostBuildGrant`, env authority, and narrowing)."""

from __future__ import annotations

from pathlib import Path
from typing import Any

import inspect_capsem
import pytest
from inspect_capsem.containers import HostBuildGrant
from inspect_capsem.containers import build_grant as bg_mod


def test_host_build_grant_env_authority_and_narrowing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv("CAPSEM_INSPECT_HOST_BUILD", raising=False)
    monkeypatch.delenv("CAPSEM_INSPECT_ALLOWED_HOST_PATHS", raising=False)
    monkeypatch.delenv("CAPSEM_INSPECT_BUILD_NETWORK", raising=False)
    monkeypatch.delenv("CAPSEM_INSPECT_BUILD_DOCKER_CONFIG", raising=False)

    assert inspect_capsem.HostBuildGrant is HostBuildGrant
    with pytest.raises(AttributeError):
        _ = inspect_capsem.NoSuchSymbol

    # Default off: task config cannot widen unset or disabled env
    assert not bg_mod.is_host_build_enabled()
    assert not bg_mod.is_host_build_enabled(True)
    assert not bg_mod.is_host_build_enabled(HostBuildGrant(enabled=True))
    monkeypatch.setenv("CAPSEM_INSPECT_HOST_BUILD", "invalid")
    with pytest.raises(ValueError, match="CAPSEM_INSPECT_HOST_BUILD"):
        bg_mod.is_host_build_enabled()

    monkeypatch.setenv("CAPSEM_INSPECT_HOST_BUILD", "1")
    assert bg_mod.is_host_build_enabled()
    assert bg_mod.is_host_build_enabled(True)
    assert not bg_mod.is_host_build_enabled(False)
    assert not bg_mod.is_host_build_enabled(HostBuildGrant(enabled=False))

    # Validation of HostBuildGrant fields and network modes
    grant_cls: Any = HostBuildGrant
    for bad_net in ("host", "bridge"):
        with pytest.raises(ValueError, match="network"):
            grant_cls(network=bad_net)
    for bad_key in ("unknown_key", "docker_config_dir"):
        with pytest.raises(ValueError, match="Unknown HostBuildGrant fields"):
            HostBuildGrant.from_value({bad_key: 1})
    with pytest.raises(TypeError, match="Expected HostBuildGrant"):
        HostBuildGrant.from_value(123)
    parsed_dict = HostBuildGrant.from_value({"allowed_contexts": f"{tmp_path},", "network": "none"})
    assert parsed_dict is not None and parsed_dict.to_dict()["network"] == "none"

    ctx_dir, other_dir, cfg_dir = tmp_path / "ctx", tmp_path / "other", tmp_path / "dcfg"
    for d in (ctx_dir, other_dir, cfg_dir):
        d.mkdir()
    df_path = ctx_dir / "Dockerfile"
    df_path.write_text("FROM scratch\n", encoding="utf-8")

    # Task allowed_contexts / allowed_host_paths cannot widen unset env
    with pytest.raises(ValueError, match="CAPSEM_INSPECT_ALLOWED_HOST_PATHS"):
        bg_mod.resolve_direct_host_build(
            {"build": str(ctx_dir)},
            host_build=HostBuildGrant(allowed_contexts=(str(ctx_dir),)),
        )
    with pytest.raises(ValueError, match="CAPSEM_INSPECT_ALLOWED_HOST_PATHS"):
        bg_mod.resolve_direct_host_build(
            {"build": str(ctx_dir)}, allowed_host_paths=(str(ctx_dir),)
        )

    monkeypatch.setenv("CAPSEM_INSPECT_ALLOWED_HOST_PATHS", f"{ctx_dir},{cfg_dir}")
    # Task cannot widen to other_dir outside env roots
    with pytest.raises(ValueError, match="not allowlisted"):
        bg_mod.resolve_direct_host_build(
            {"build": str(ctx_dir)}, allowed_host_paths=(str(other_dir),)
        )
    with pytest.raises(ValueError, match="not allowlisted"):
        bg_mod.resolve_direct_host_build(
            {"build": str(ctx_dir)},
            host_build=HostBuildGrant(allowed_contexts=(str(other_dir),)),
        )

    # Network ceiling: default env is 'none', so task network='default' is refused
    with pytest.raises(ValueError, match="exceeds operator ceiling"):
        bg_mod.resolve_direct_host_build(
            {"build": str(ctx_dir)},
            host_build=HostBuildGrant(network="default"),
        )
    monkeypatch.setenv("CAPSEM_INSPECT_BUILD_NETWORK", "host")
    with pytest.raises(ValueError, match="forbids network='host'"):
        bg_mod.resolve_direct_host_build({"build": str(ctx_dir)})
    monkeypatch.setenv("CAPSEM_INSPECT_BUILD_NETWORK", "bad")
    with pytest.raises(ValueError, match="CAPSEM_INSPECT_BUILD_NETWORK"):
        bg_mod.resolve_direct_host_build({"build": str(ctx_dir)})

    monkeypatch.setenv("CAPSEM_INSPECT_BUILD_NETWORK", "default")
    # Task can narrow default -> none, or match default
    spec_narrow = bg_mod.resolve_direct_host_build(
        {"build": str(ctx_dir)}, host_build=HostBuildGrant(network="none")
    )
    assert spec_narrow is not None and spec_narrow["network"] == "none"

    # Docker config dir env authority (CAPSEM_INSPECT_BUILD_DOCKER_CONFIG)
    monkeypatch.setenv("CAPSEM_INSPECT_BUILD_DOCKER_CONFIG", str(other_dir))
    with pytest.raises(ValueError, match="not within CAPSEM_INSPECT_ALLOWED_HOST_PATHS"):
        bg_mod.resolve_direct_host_build({"build": str(ctx_dir)})
    monkeypatch.setenv("CAPSEM_INSPECT_BUILD_DOCKER_CONFIG", str(tmp_path / "missing_dcfg"))
    with pytest.raises(ValueError, match="directory not found"):
        bg_mod.resolve_direct_host_build({"build": str(ctx_dir)})
    monkeypatch.setenv("CAPSEM_INSPECT_BUILD_DOCKER_CONFIG", str(cfg_dir))
    spec_ok = bg_mod.resolve_direct_host_build(
        {"build": str(ctx_dir), "dockerfile": "Dockerfile"},
        host_build=HostBuildGrant(
            allowed_contexts=(str(ctx_dir),),
            network="default",
        ),
    )
    assert spec_ok is not None
    assert spec_ok["docker_config_dir"] == str(cfg_dir.resolve())
    assert spec_ok["network"] == "default"


def test_prepare_compose_build_service_validation(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    assert bg_mod.resolve_direct_host_build({}) is None
    for bad_svc, match in (
        ({"dockerfile": ""}, "non-empty path string"),
        ({"build": "", "dockerfile": "Dockerfile"}, "context path string or mapping"),
        ({"build": ""}, "non-empty context path string"),
        ({"build": {"context": ""}}, "build.context"),
        ({"build": {"dockerfile": 123}}, "build.dockerfile"),
        ({"build": "git@github.com:org/repo.git"}, "Remote build context"),
    ):
        with pytest.raises(ValueError, match=match):
            bg_mod.prepare_compose_build_service(bad_svc)
    merged, stanza = bg_mod.prepare_compose_build_service(
        {"build": {"target": "s1"}, "dockerfile": "Custom.df"}
    )
    assert stanza == "build" and merged["build"] == {
        "dockerfile": "Custom.df",
        "target": "s1",
        "context": ".",
    }
    monkeypatch.setenv("CAPSEM_INSPECT_HOST_BUILD", "1")
    monkeypatch.setenv("CAPSEM_INSPECT_ALLOWED_HOST_PATHS", str(tmp_path))
    with pytest.raises(ValueError, match="disabled by default"):
        bg_mod.resolve_direct_host_build({"build": str(tmp_path)}, host_build=False)
