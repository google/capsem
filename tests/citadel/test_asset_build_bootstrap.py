"""Citadel guard: hosted asset builders provision the canonical host first."""

from __future__ import annotations

from collections.abc import Mapping
from enum import StrEnum
from pathlib import Path
from typing import cast

import yaml
from helpers.workflow_contract import canonical_shell_commands

PROJECT_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = PROJECT_ROOT / ".github" / "workflows"

ASSET_BUILD_BOOTSTRAP_RATIONALE = """\
Every hosted workflow job that builds VM assets must run the canonical bootstrap
before entering capsem-gate. A fresh asset runner otherwise reaches Doctor
without Bubblewrap, b3sum, the config-owned Rust targets/components and Cargo
tools, or the Linux musl compiler. The local complete gate can be green while
release CI fails immediately because jobs do not share their host filesystem.

The bootstrap command must be a fail-closed step with asset self-building
disabled: bootstrap materializes host prerequisites, while the following
profile-owned build-assets plan remains the sole owner of the artifact bytes.

See skills/build-images/SKILL.md and skills/dev-ci/SKILL.md.
"""


class Key(StrEnum):
    JOBS = "jobs"
    STEPS = "steps"
    RUN = "run"
    ENV = "env"
    NAME = "name"
    CONTINUE_ON_ERROR = "continue-on-error"


class Recipe(StrEnum):
    #: The private whole-cohort builder, and the public per-arch verb that
    #: workflows call. `_build-kernel` and `_build-rootfs` folded into the
    #: latter when CI stopped reaching into private recipes.
    BUILD_ASSETS_PRIVATE = "_build-assets"
    BUILD_ASSETS = "build-assets"


class Bootstrap(StrEnum):
    SHELL = "sh"
    SCRIPT = "bootstrap.sh"
    YES = "--yes"
    SKIP_ASSETS = "CAPSEM_SKIP_ASSET_CHECK"
    SKIP_VALUE = "1"


ASSET_RECIPES = frozenset(Recipe)


def _string_mapping(value: object) -> Mapping[str, object] | None:
    if not isinstance(value, Mapping) or not all(isinstance(key, str) for key in value):
        return None
    return cast(Mapping[str, object], value)


def _is_asset_build(command: tuple[str, ...]) -> bool:
    return len(command) >= 2 and command[0] == "just" and command[1] in ASSET_RECIPES


def _is_canonical_bootstrap(step: Mapping[str, object]) -> bool:
    if Key.CONTINUE_ON_ERROR in step:
        return False
    env = _string_mapping(step.get(Key.ENV))
    if env is None or env.get(Bootstrap.SKIP_ASSETS) != Bootstrap.SKIP_VALUE:
        return False
    body = step.get(Key.RUN)
    if not isinstance(body, str):
        return False
    expected = (Bootstrap.SHELL, Bootstrap.SCRIPT, Bootstrap.YES)
    return canonical_shell_commands(body) == (expected,)


def _unprovisioned_asset_steps(document: Mapping[str, object]) -> list[str]:
    offenders: list[str] = []
    jobs = _string_mapping(document.get(Key.JOBS))
    assert jobs is not None
    for job_name, job in jobs.items():
        job = _string_mapping(job)
        assert job is not None
        provisioned = False
        steps = job.get(Key.STEPS)
        if steps is None:
            continue
        assert isinstance(steps, list)
        for step in steps:
            step = _string_mapping(step)
            assert step is not None
            provisioned = provisioned or _is_canonical_bootstrap(step)
            body = step.get(Key.RUN)
            if not isinstance(body, str):
                continue
            if not provisioned and any(
                _is_asset_build(command) for command in canonical_shell_commands(body)
            ):
                offenders.append(f"{job_name}:{step.get(Key.NAME, '<unnamed>')}")
    return offenders


def test_every_workflow_asset_builder_bootstraps_its_fresh_host() -> None:
    asset_steps = 0
    offenders: list[str] = []
    for path in sorted(WORKFLOWS.glob("*.yaml")):
        document = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        found = _unprovisioned_asset_steps(document)
        offenders.extend(f"{path.name}:{offender}" for offender in found)
        for job in (document.get(Key.JOBS) or {}).values():
            for step in job.get(Key.STEPS) or ():
                body = step.get(Key.RUN)
                if isinstance(body, str):
                    asset_steps += sum(
                        _is_asset_build(command) for command in canonical_shell_commands(body)
                    )
    assert asset_steps, "no hosted asset-build command found; guard would be vacuous"
    assert not offenders, ASSET_BUILD_BOOTSTRAP_RATIONALE + f"\noffenders: {offenders}"


def _fixture(*steps: Mapping[str, object]) -> dict[str, object]:
    return {Key.JOBS: {"assets": {Key.STEPS: list(steps)}}}


def _bootstrap_step(run: str = "sh bootstrap.sh --yes") -> dict[str, object]:
    return {
        Key.NAME: "Bootstrap complete asset host",
        Key.ENV: {Bootstrap.SKIP_ASSETS: Bootstrap.SKIP_VALUE},
        Key.RUN: run,
    }


def _build_step() -> dict[str, object]:
    return {Key.NAME: "Build assets", Key.RUN: "just build-assets x86_64 code"}


def test_guard_rejects_missing_late_or_neutralized_bootstrap() -> None:
    assert _unprovisioned_asset_steps(_fixture(_build_step()))
    assert _unprovisioned_asset_steps(_fixture(_build_step(), _bootstrap_step()))
    assert _unprovisioned_asset_steps(
        _fixture(_bootstrap_step("sh bootstrap.sh --yes || true"), _build_step())
    )
    assert _unprovisioned_asset_steps(
        _fixture(_bootstrap_step("set +e\nsh bootstrap.sh --yes"), _build_step())
    )
    advisory = _bootstrap_step()
    advisory[Key.CONTINUE_ON_ERROR] = False
    assert _unprovisioned_asset_steps(_fixture(advisory, _build_step()))


def test_guard_accepts_fail_closed_bootstrap_before_asset_build() -> None:
    assert _unprovisioned_asset_steps(_fixture(_bootstrap_step(), _build_step())) == []
