"""A live channel must be proven to resolve even when nobody is deploying.

`check-release-site-contract.py` already fetches every artifact a manifest
references -- including GitHub release downloads -- and verifies size and
sha256. That check is thorough. It runs in exactly one place: the channel
deploy.

So the release system can only detect a broken channel at the moment it
publishes a new one. Anything that breaks a *published* channel from outside a
deploy goes unnoticed: a deleted GitHub release, an artifact garbage-collected
by a retention policy, a CDN serving an HTML fallback with HTTP 200. Users hit
it first, and the release that "worked" is the one that stops working later.

Deleting the 1.x GitHub releases proved the failure mode: stable 1.0.143 kept
serving a manifest whose twenty artifact URLs had all become 404s, and every
gate stayed green because no gate was looking.
"""

from __future__ import annotations

import importlib.util
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import yaml
from capsem_builder.release import runtime_preflight_manifest as channel_resolver

PROJECT_ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = PROJECT_ROOT / ".github" / "workflows"
VALIDATOR = "check-release-site-contract.py"


def _validator_module() -> Any:
    path = PROJECT_ROOT / "scripts" / VALIDATOR
    spec = importlib.util.spec_from_file_location("live_channel_validator", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


@dataclass(frozen=True)
class _Check:
    name: str
    ok: bool
    detail: str


class _Checker:
    BLAKE3_IMPORT_ERROR = None

    def __init__(self, broken: set[str] | None = None) -> None:
        self.broken = broken or set()
        self.checked: list[str] = []

    def check_release_site_contract(self, _site: str, channel: str) -> _Check:
        self.checked.append(channel)
        ok = channel not in self.broken
        return _Check("release contract", ok, "resolved" if ok else "broken reference")


def _workflows() -> dict[str, dict]:
    loaded = {}
    for path in sorted(WORKFLOWS.glob("*.yaml")):
        loaded[path.name] = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
    assert loaded, f"no workflows found under {WORKFLOWS}"
    return loaded


def _triggers(workflow: dict) -> dict:
    # `on` is parsed as the boolean True by YAML 1.1 unless quoted.
    return workflow.get("on") or workflow.get(True) or {}


def _runs_validator(path: Path) -> bool:
    return VALIDATOR in path.read_text(encoding="utf-8")


def test_a_scheduled_workflow_validates_the_live_channels() -> None:
    scheduled = [
        name
        for name, workflow in _workflows().items()
        if "schedule" in _triggers(workflow) and _runs_validator(WORKFLOWS / name)
    ]

    assert scheduled, (
        f"no scheduled workflow runs {VALIDATOR}, so a published channel is only "
        "checked while deploying a new one -- a deleted or expired artifact stays "
        "invisible until the next release"
    )


def test_the_live_channel_watch_covers_every_channel() -> None:
    """The typed catalog decides which first-party channels are published."""
    watchers = [
        WORKFLOWS / name
        for name, workflow in _workflows().items()
        if "schedule" in _triggers(workflow) and _runs_validator(WORKFLOWS / name)
    ]
    assert watchers, "no live channel watch to inspect"

    covered = " ".join(path.read_text(encoding="utf-8") for path in watchers)
    assert "--catalog-members" in covered
    assert "--channel stable" not in covered
    assert "--channel nightly" not in covered


def test_catalog_watch_skips_absent_and_checks_published_members() -> None:
    validator = _validator_module()
    checker = _Checker()

    def resolve(**kwargs: Any) -> channel_resolver.ChannelResolution:
        channel = kwargs["channel"]
        state = (
            channel_resolver.ChannelState.PUBLISHED
            if channel is channel_resolver.retirement.FirstPartyChannel.STABLE
            else channel_resolver.ChannelState.ABSENT
        )
        return channel_resolver.ChannelResolution(channel, state, state.value)

    result = validator.validate_catalog_members(
        release_site="file:///release-site",
        attempts=1,
        delay_seconds=0,
        checker=checker,
        resolver=resolve,
    )

    assert result == 0
    assert checker.checked == ["stable"]


def test_catalog_watch_fails_closed_on_invalid_state() -> None:
    validator = _validator_module()
    checker = _Checker()

    def resolve(**kwargs: Any) -> channel_resolver.ChannelResolution:
        channel = kwargs["channel"]
        return channel_resolver.ChannelResolution(
            channel,
            channel_resolver.ChannelState.INVALID,
            "HTML fallback",
        )

    result = validator.validate_catalog_members(
        release_site="file:///release-site",
        attempts=1,
        delay_seconds=0,
        checker=checker,
        resolver=resolve,
    )

    assert result == 1
    assert checker.checked == []


def test_catalog_watch_still_exposes_broken_retired_references() -> None:
    validator = _validator_module()
    checker = _Checker({"stable"})

    def resolve(**kwargs: Any) -> channel_resolver.ChannelResolution:
        channel = kwargs["channel"]
        state = (
            channel_resolver.ChannelState.RETIRED
            if channel is channel_resolver.retirement.FirstPartyChannel.STABLE
            else channel_resolver.ChannelState.ABSENT
        )
        return channel_resolver.ChannelResolution(channel, state, state.value)

    result = validator.validate_catalog_members(
        release_site="file:///release-site",
        attempts=1,
        delay_seconds=0,
        checker=checker,
        resolver=resolve,
    )

    assert result == 1
    assert checker.checked == ["stable"]


def test_the_watch_can_be_run_on_demand() -> None:
    """An operator must be able to answer "is the channel healthy right now?".

    Waiting for the next scheduled tick to confirm a fix is how a five-minute
    verification becomes an hour.
    """
    watchers = [
        (name, workflow)
        for name, workflow in _workflows().items()
        if "schedule" in _triggers(workflow) and _runs_validator(WORKFLOWS / name)
    ]
    assert watchers, "no live channel watch to inspect"

    for name, workflow in watchers:
        assert "workflow_dispatch" in _triggers(workflow), (
            f"{name} watches the live channels but cannot be triggered by hand"
        )
