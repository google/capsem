"""Package imports, entry-point registration, and `CapsemSandboxConfig` tests."""

from __future__ import annotations

import tomllib
from pathlib import Path
from typing import Any, cast

import inspect_capsem
import pytest
from inspect_ai.util._sandbox.registry import registry_find_sandboxenv
from inspect_capsem import CapsemSandboxConfig, CapsemSandboxEnvironment
from inspect_capsem._controller import SdkCapsemController, is_root_user_spec
from inspect_capsem._exec import _format_exec_command
from inspect_capsem.config import coerce_config
from pydantic import ValidationError


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
        cpu_count=2,
        ram_gb=4,
        working_dir="/tmp",
        environment={"FOO": "bar"},
        user="developer",
    )
    assert cfg.cpu_count == 2
    assert hash(default_cfg) != hash(cfg)
    restored = CapsemSandboxEnvironment.config_deserialize(cfg.model_dump(mode="json"))
    assert isinstance(restored, CapsemSandboxConfig)
    assert restored == cfg


def test_coerce_config_and_unsupported_knobs() -> None:
    assert coerce_config(None) == CapsemSandboxConfig()
    cfg = CapsemSandboxConfig(cpu_count=2)
    assert coerce_config(cfg) is cfg
    assert coerce_config({"cpu_count": 2}) == CapsemSandboxConfig(cpu_count=2)
    with pytest.raises(NotImplementedError, match="template"):
        coerce_config({"template": "code"})
    with pytest.raises(NotImplementedError, match="allow_domains"):
        coerce_config({"allow_domains": ["pypi.org"]})
    with pytest.raises(NotImplementedError, match="host_workspace_dir"):
        coerce_config({"host_workspace_dir": "/tmp/ws"})
    with pytest.raises(ValidationError):
        coerce_config({"pool_endpoint": "http://127.0.0.1:9999"})
    with pytest.raises(TypeError, match="Unsupported Capsem sandbox config type"):
        coerce_config(cast(Any, object()))


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
    assert "setpriv --reuid=0 --regid=1000 --clear-groups /bin/bash -c" in cmd_0_1000
    for spec, expected in (
        ("1000:users", ("_uid=1000;", "getent group users")),
        ("alice:1000", ("_uid=$(id -u alice 2>/dev/null)", "_gid=1000;")),
        ("alice:users", ("_uid=$(id -u alice 2>/dev/null)", "getent group users")),
    ):
        cmd, _ = _format_exec_command("id", effective_cwd="/", env=None, user=spec, timeout=None)
        assert all(part in cmd for part in expected)
        assert 'setpriv --reuid="$_uid" --regid="$_gid" --clear-groups /bin/bash -c' in cmd


def test_namespaced_basetemp_isolates_shared_gate_basetemp(
    tmp_path: Path,
) -> None:
    from tests.conftest import _namespaced_basetemp, pytest_configure

    assert _namespaced_basetemp(None, {}) is None
    assert _namespaced_basetemp("/tmp/base", {"PYTEST_XDIST_WORKER": "gw0"}) == "/tmp/base"
    assert _namespaced_basetemp("/tmp/base", {}) == "/tmp/base/inspect-capsem"
    assert (
        _namespaced_basetemp(
            "/tmp/base", {"CAPSEM_TEST_RUN_ID": "fast.integrations.inspect-ai.tests"}
        )
        == "/tmp/base/fast-integrations-inspect-ai-tests"
    )

    shared = tmp_path / "shared-pytest"
    sentinel = shared / "citadel" / "worker.txt"
    sentinel.parent.mkdir(parents=True)
    sentinel.write_text("keep", encoding="utf-8")

    class _DummyOption:
        basetemp = str(shared)

    class _DummyTmpFactory:
        _given_basetemp = shared.resolve()
        _basetemp: Path | None = None

    class _DummyConfig:
        option = _DummyOption()
        _tmp_path_factory = _DummyTmpFactory()

        def addinivalue_line(self, name: str, line: str) -> None:
            pass

    cfg = _DummyConfig()
    pytest_configure(cast(pytest.Config, cfg))
    assert cfg.option.basetemp != str(shared)
    assert cfg._tmp_path_factory._given_basetemp == Path(cfg.option.basetemp).resolve()
    assert sentinel.read_text(encoding="utf-8") == "keep"
