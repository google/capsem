"""The sole locked Python project used by every engineering command."""

from __future__ import annotations

from .config import GateConfig


def uv_run(config: GateConfig, *arguments: object) -> list[str]:
    """Run arguments through the build-system project without lock mutation."""
    return [
        "uv",
        "run",
        "--project",
        config.suites.pytest.build_system_project,
        "--frozen",
        *(str(argument) for argument in arguments),
    ]


def uv_run_installed(config: GateConfig, *arguments: object) -> list[str]:
    """Run against a conventional locked install, never an editable finder."""
    return [
        "uv",
        "run",
        "--isolated",
        "--no-editable",
        "--reinstall-package",
        config.suites.pytest.project_distribution,
        "--no-build-isolation-package",
        config.suites.pytest.project_distribution,
        "--offline",
        "--project",
        config.suites.pytest.build_system_project,
        "--frozen",
        *(str(argument) for argument in arguments),
    ]


def pytest(config: GateConfig, *arguments: object) -> list[str]:
    """Run pytest with the sole project's configuration selected explicitly."""
    return uv_run(
        config,
        "python",
        "-m",
        "pytest",
        "-c",
        config.suites.pytest.project_manifest,
        *arguments,
    )
