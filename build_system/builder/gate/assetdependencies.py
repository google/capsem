"""Visible per-architecture dependency materialization for runtime asset builds."""

from __future__ import annotations

from collections.abc import Iterable
from enum import StrEnum
from pathlib import PurePosixPath

from .actions import Run
from .config import GateConfig
from .errors import GateError
from .execution import Kind, Needs, Speed, Step, step


class DependencyOperation(StrEnum):
    """The closed private-backend operations a gate may request."""

    MATERIALIZE = "--materialize-dependencies"
    REQUIRE = "--require-dependencies"


def templates_for(config: GateConfig, requested: str) -> tuple[str, ...]:
    """Expand the public `all` selector into its config-owned dependency set."""
    return config.imagebuild.lane_templates if requested == "all" else (requested,)


def dependency_step(
    config: GateConfig,
    arches: Iterable[str],
    templates: Iterable[str] | None = None,
    *,
    label: str = "asset-dependencies",
) -> Step:
    """One resumable frontier between network acquisition and source builds."""
    selected_arches = tuple(arches)
    selected = config.imagebuild.lane_templates if templates is None else tuple(templates)
    return step(
        label,
        *materialize_actions(config, selected_arches, selected),
        # Both claims: it pulls base images through the daemon and interleaves
        # `cargo run -p capsem-admin image workspace` between the pulls.
        contends=(config.exclusive("docker_daemon"), config.exclusive("workspace_binaries")),
        carry_checks=require_actions(config, selected_arches, selected),
        kind=Kind.PACKAGE,
        needs=frozenset({Needs.DOCKER, Needs.DISK}),
        speed=Speed.SLOW,
    )


def request_step(config: GateConfig, arches: Iterable[str], requested: str) -> Step:
    return dependency_step(config, arches, templates_for(config, requested))


def _scope(
    config: GateConfig,
    arches: Iterable[str],
    templates: Iterable[str],
) -> tuple[tuple[str, ...], tuple[str, ...]]:
    selected_arches = tuple(config.arch(name).name for name in arches)
    selected_templates = tuple(templates)
    unknown = sorted(set(selected_templates) - set(config.imagebuild.lane_templates))
    if unknown:
        raise GateError(f"unsupported asset dependency templates: {', '.join(unknown)}")
    return selected_arches, selected_templates


def _workspace(config: GateConfig, arch: str) -> str:
    return config.imagebuild.workspace_root.format(arch=arch)


def materialize_actions(
    config: GateConfig,
    arches: Iterable[str],
    templates: Iterable[str],
) -> tuple[Run, ...]:
    """Describe every workspace write and network-open helper build."""
    selected_arches, selected_templates = _scope(config, arches, templates)
    actions: list[Run] = []
    for arch in selected_arches:
        workspace = _workspace(config, arch)
        actions.append(
            Run(
                [
                    *config.imagebuild.workspace_admin,
                    "--config-root",
                    config.imagebuild.config_root,
                    "--guest-dir",
                    config.imagebuild.guest_dir,
                    "--output",
                    workspace,
                    "--arch",
                    arch,
                    "--json",
                ]
            )
        )
        actions.extend(
            _backend_action(config, workspace, arch, template, DependencyOperation.MATERIALIZE)
            for template in selected_templates
        )
    return tuple(actions)


def require_actions(
    config: GateConfig,
    arches: Iterable[str],
    templates: Iterable[str],
) -> tuple[Run, ...]:
    """Describe exact-image checks for a diagnostically carried frontier."""
    selected_arches, selected_templates = _scope(config, arches, templates)
    return tuple(
        _backend_action(
            config,
            _workspace(config, arch),
            arch,
            template,
            DependencyOperation.REQUIRE,
        )
        for arch in selected_arches
        for template in selected_templates
    )


def _backend_action(
    config: GateConfig,
    workspace: str,
    arch: str,
    template: str,
    operation: DependencyOperation,
) -> Run:
    return Run(
        [
            *config.imagebuild.dependency_backend,
            str(PurePosixPath(workspace) / config.imagebuild.workspace_guest_dir),
            "--arch",
            arch,
            "--template",
            template,
            operation.value,
        ]
    )
