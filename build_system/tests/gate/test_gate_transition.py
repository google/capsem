"""The installed public-to-candidate transition runs before any release (#280).

Three stable 0.6.4 qualifications failed one hosted run at a time on what
only an installed public release updating itself can show: a profile key 0.6.3
refuses, a rule 0.6.3 cannot reload, a ledger 0.6.4 would not open. These hold
that the local rehearsal replays that transition, with the release lane's own
script and inputs, in the disposable install container.
"""

from __future__ import annotations

from pathlib import Path

from capsem_builder.gate import transition
from capsem_builder.gate.actions import Script
from capsem_builder.gate.config import for_root
from capsem_builder.gate.execution import Needs
from helpers.gate import RecordingRunner, built_command

PROJECT_ROOT = Path(__file__).resolve().parents[3]
CONFIG = for_root(PROJECT_ROOT)
COMMIT = "f" * 40


def test_the_rehearsal_replays_the_installed_transition_last() -> None:
    labels = list(built_command(PROJECT_ROOT, "test-rehearsal")._describe().labels)
    order = [
        "rehearsal.package",
        "rehearsal.transition.before-packages",
        "rehearsal.transition.before-profiles",
        "rehearsal.transition.before-verify",
        "rehearsal.transition",
    ]
    assert [labels.index(label) for label in order] == sorted(labels.index(label) for label in order)
    assert labels[-1] == "rehearsal.transition"


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
        assert CONFIG.modules.transition_input_cache in rendered


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

    (command, env), = shells
    pairing = CONFIG.modules.release_pairing
    channel = CONFIG.modules.rehearsal_channel
    assert env[pairing.channel] == env[pairing.baseline_channel] == channel
    assert env[pairing.transition] == "auto"
    assert env[pairing.before_manifest].endswith(f"{transition.PROFILES}/{CONFIG.install.manifest_name}")
    assert env[pairing.after_manifest].endswith(
        CONFIG.modules.rehearsal_after_manifest.format(channel=channel)
    )
    assert env[pairing.after_profile_inputs].endswith(CONFIG.modules.rehearsal_inputs_dir)
    assert f'--before-package "{before}"' in command
    assert f"--source-commit {COMMIT}" in command
    assert '--evidence-dir "/src/evidence"' in command
    assert CONFIG.install.suite.glowup_script in command


def test_a_first_release_has_nothing_to_update_from(monkeypatch, tmp_path) -> None:
    gate, shells = _gate(monkeypatch)
    gate._before = tmp_path
    monkeypatch.setattr(gate._container, "runtime_options", lambda: [])
    gate._container.boots_a_guest = True
    gate.run()
    assert shells == []
