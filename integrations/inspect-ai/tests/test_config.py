"""Package imports, entry-point registration, and `CapsemSandboxConfig` tests."""

from __future__ import annotations

from inspect_ai.util._sandbox.registry import registry_find_sandboxenv
from inspect_capsem import (
    CapsemSandboxConfig,
    CapsemSandboxEnvironment,
    SdkCapsemController,
)


def test_standalone_package_imports_and_registration() -> None:
    sandbox_cls = registry_find_sandboxenv("capsem")
    assert sandbox_cls is CapsemSandboxEnvironment
    default_cfg = CapsemSandboxConfig()
    assert default_cfg.execution_mode == "vm"
    assert default_cfg.template == "code"
    assert CapsemSandboxEnvironment.config_deserialize(default_cfg.model_dump(mode="json")) == (
        default_cfg
    )
    cfg = CapsemSandboxConfig(
        execution_mode="container",
        template="harbor",
        image="ubuntu:24.04",
        cpu_count=2,
        ram_gb=4,
        working_dir="/tmp",
        command=("tail", "-f", "/dev/null"),
    )
    assert cfg.execution_mode == "container"
    assert cfg.template == "harbor"
    assert hash(default_cfg) != hash(cfg)
    restored = CapsemSandboxEnvironment.config_deserialize(cfg.model_dump(mode="json"))
    assert isinstance(restored, CapsemSandboxConfig)
    assert restored == cfg
    assert SdkCapsemController is not None
