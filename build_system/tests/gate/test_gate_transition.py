"""The installed public-to-candidate transition runs before any release (#280).

Three stable 0.6.4 qualifications failed one hosted run at a time on what
only an installed public release updating itself can show: a profile key 0.6.3
refuses, a rule 0.6.3 cannot reload, a ledger 0.6.4 would not open. These hold
that the local rehearsal replays that transition, with the release lane's own
script and inputs, in the disposable install container.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from capsem_builder.gate import transition
from capsem_builder.gate.actions import Script
from capsem_builder.gate.config import for_root
from capsem_builder.gate.execution import Needs
from capsem_builder.release.tools.fetch_release_artifacts import fetch_release_inputs
from capsem_builder.release.tools.verify_release_inputs import verify_release_inputs
from helpers.gate import RecordingRunner, built_command

PROJECT_ROOT = Path(__file__).resolve().parents[3]
CONFIG = for_root(PROJECT_ROOT)
COMMIT = "f" * 40


def test_just_test_replays_the_installed_transition_after_the_rehearsal() -> None:
    """Its own phase: the rehearsal stays a local cohort that fetches nothing."""
    labels = list(built_command(PROJECT_ROOT, "candidate")._describe().labels)
    order = [
        "rehearsal.package",
        "transition.before-packages",
        "transition.before-runtime",
        "transition.before-verify",
        "transition.installed",
        "recipes",
    ]
    positions = [labels.index(label) for label in order]
    assert positions == sorted(positions), dict(zip(order, positions, strict=True))


def test_the_before_state_is_fetched_over_the_release_egress() -> None:
    """The sandbox forbids a mid-run fetch; the egress is the sanctioned way out."""
    fetches = transition.fetch_before(CONFIG)[:2]
    for fetched in fetches:
        (action,) = fetched.actions
        assert isinstance(action, Script)
        assert Needs.NETWORK in fetched.needs
        rendered = action.render()
        assert rendered.endswith("[outside kernel sandbox]")
        assert CONFIG.package.default_manifest_url in rendered
        assert CONFIG.modules.transition.input_cache in rendered


def test_public_packages_may_precede_the_first_runtime(tmp_path: Path) -> None:
    """The deployed pre-0.7 graph has packages and profiles but no runtime yet."""
    manifest = tmp_path / "manifest.json"
    payload = json.dumps({
        "channel": "stable",
        "packages": [{"version": "0.6.6", "status": "current"}],
        "profiles": {"code": {"status": "current"}},
    }).encode()
    manifest.write_bytes(payload)
    (action,) = transition.fetch_before(CONFIG)[1].actions
    assert isinstance(action, Script)
    output = tmp_path / "before-runtime"
    report = fetch_release_inputs(
        manifest.as_uri(), "runtime", output,
        allow_empty_runtime="--allow-empty-runtime" in action.render().split(),
        architecture=CONFIG.host_arch().name,
    )
    assert report["artifacts"] == []
    assert report["allow_empty_runtime"] is True
    assert (output / "manifest.json").read_bytes() == payload
    verify_release_inputs(output)
    # Candidate/runtime consumers retain their strict default.
    with pytest.raises(ValueError, match="contains no runtime"):
        fetch_release_inputs(manifest.as_uri(), "runtime", tmp_path / "strict")


def test_empty_before_runtime_permission_keeps_malformed_runtime_fail_closed(tmp_path: Path) -> None:
    manifest = tmp_path / "manifest.json"
    manifest.write_text(json.dumps({"channel": "stable", "packages": [], "runtime": {}}))
    (action,) = transition.fetch_before(CONFIG)[1].actions
    assert isinstance(action, Script)
    with pytest.raises(ValueError, match="runtime has no architectures"):
        fetch_release_inputs(
            manifest.as_uri(), "runtime", tmp_path / "before-runtime",
            allow_empty_runtime="--allow-empty-runtime" in action.render().split(),
        )


def _gate(monkeypatch) -> tuple[transition.TransitionGate, list[tuple[str, dict]]]:
    shells: list[tuple[str, dict]] = []
    monkeypatch.setattr(
        transition.Docker,
        "shell",
        lambda self, container, command, **kwargs: shells.append((command, kwargs["env"])),
    )
    return transition.TransitionGate(RecordingRunner(PROJECT_ROOT), source_commit=COMMIT), shells


def test_the_glowup_gets_the_release_lanes_pairing(monkeypatch) -> None:
    gate, shells = _gate(monkeypatch)
    before = Path("/before/Capsem_0.6.3_amd64.deb")
    gate._glowup(before, "/src/evidence")

    ((command, env),) = shells
    pairing = CONFIG.modules.release_pairing
    channel = CONFIG.modules.rehearsal_channel
    assert env[pairing.channel] == env[pairing.baseline_channel] == channel
    assert env[pairing.transition] == "auto"
    assert env[pairing.before_manifest].endswith(
        f"{transition.RUNTIME}/{CONFIG.install.manifest_name}"
    )
    assert env[pairing.after_manifest].endswith(
        CONFIG.modules.rehearsal_after_manifest.format(channel=channel)
    )
    assert env[pairing.before_release_inputs].endswith(transition.RUNTIME)
    assert env[pairing.after_release_inputs].endswith(CONFIG.modules.rehearsal_inputs_dir)
    # 0.7 has no profiles: the glow-up takes neither a config root nor a profile policy.
    assert "--config-root" not in command and "--profile" not in command
    assert f'--before-package "{before}"' in command
    assert f"--source-commit {COMMIT}" in command
    assert '--evidence-dir "/src/evidence"' in command
    assert CONFIG.install.suite.glowup_script in command


def test_a_first_release_has_nothing_to_update_from(monkeypatch, tmp_path) -> None:
    gate, shells = _gate(monkeypatch)
    gate._before = tmp_path
    monkeypatch.setattr(gate._container, "runtime_options", list)
    gate._container.boots_a_guest = True
    gate.run()
    assert shells == []
