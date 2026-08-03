"""What a gate run is proving, and against which bytes.

Three modules each decided independently whether they were in a release lane,
by reading a different environment variable. Nothing checked that the three
answers agreed, so dropping one `GITHUB_ENV` line did not fail the release --
it built a plan that verified manifest-selected assets and then rebuilt the
package from source, and proved the wrong bytes with a green result.

The states are indivisible here, and every partial combination is refused
while the plan is still being built.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PROJECT_ROOT / "tests"))

from capsem.gate import config as gate_config  # noqa: E402
from capsem.gate.errors import GateError  # noqa: E402
from capsem.gate.qualification import Mode, Qualification  # noqa: E402

CONFIG = gate_config.load(PROJECT_ROOT)
SETTINGS = CONFIG.modules

INPUT_DIR = SETTINGS.release_input_dir
PACKAGE = SETTINGS.release_package
PROFILE = SETTINGS.release_profile

#: A value for each variable, so a case is written as the set of names present.
VALUES = {
    INPUT_DIR: "target/candidate-profile-inputs",
    PACKAGE: "dist/capsem_0.6.0_arm64.deb",
    PROFILE: "code",
}


def environment(*present: str) -> dict[str, str]:
    return {name: VALUES[name] for name in present}


# ---------------------------------------------------------------------------
# The complete table: eight combinations, three of which are a release contract
# ---------------------------------------------------------------------------

VALID = {
    (): Mode.LOCAL,
    (INPUT_DIR, PACKAGE): Mode.BINARY_RELEASE,
    (INPUT_DIR, PACKAGE, PROFILE): Mode.PROFILE_RELEASE,
}

PARTIAL = (
    (INPUT_DIR,),
    (PACKAGE,),
    (PROFILE,),
    (INPUT_DIR, PROFILE),
    (PACKAGE, PROFILE),
)


@pytest.mark.parametrize(("present", "mode"), sorted(VALID.items()))
def test_the_three_valid_release_states_parse(present: tuple[str, ...], mode: Mode) -> None:
    qualification = Qualification.from_environment(CONFIG, environment(*present))

    assert qualification.mode is mode
    assert qualification.pulled is (mode is not Mode.LOCAL)


@pytest.mark.parametrize("present", PARTIAL)
def test_every_partial_release_environment_is_refused(present: tuple[str, ...]) -> None:
    """Naming both sides, because the operator has to fix the missing one."""
    with pytest.raises(GateError) as raised:
        Qualification.from_environment(CONFIG, environment(*present))

    message = str(raised.value)
    for name in present:
        assert name in message, f"{name} is set and the refusal never mentions it"
    for name in set(VALUES) - set(present):
        if name is PROFILE and INPUT_DIR in present and PACKAGE in present:
            continue
        assert name in message, f"{name} is missing and the refusal never mentions it"


def test_an_empty_variable_counts_as_absent() -> None:
    """`echo "VAR=" >> $GITHUB_ENV` sets it to the empty string.

    Treating that as present is how a release lane would be told to verify an
    input directory named "".
    """
    assert Qualification.from_environment(CONFIG, {INPUT_DIR: "", PACKAGE: ""}).mode is Mode.LOCAL

    with pytest.raises(GateError):
        Qualification.from_environment(CONFIG, {INPUT_DIR: VALUES[INPUT_DIR], PACKAGE: "   "})


def test_the_profile_release_carries_its_profile_and_the_binary_release_does_not() -> None:
    binary = Qualification.from_environment(CONFIG, environment(INPUT_DIR, PACKAGE))
    profile = Qualification.from_environment(CONFIG, environment(INPUT_DIR, PACKAGE, PROFILE))

    assert binary.profile is None
    assert profile.profile == VALUES[PROFILE]
    assert binary.input_dir == profile.input_dir == VALUES[INPUT_DIR]
    assert binary.package == profile.package == VALUES[PACKAGE]


# ---------------------------------------------------------------------------
# The plans those states produce, built fresh -- the cache in `helpers.gate`
# has no environment in its key, and hid exactly this during review
# ---------------------------------------------------------------------------


def planned(module: str, qualification: Qualification) -> str:
    """One module's plan under an explicit qualification, freshly built."""
    from helpers.gate import RecordingRunner

    from capsem.gate import cli  # noqa: F401 - registers every command
    from capsem.gate.command import GateCommand

    command = GateCommand.registry[module](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(dry_run=False, graph=False, timing=False),
        qualification=qualification,
    )
    return command._describe().describe()


LOCAL = Qualification.from_environment(CONFIG, {})
BINARY = Qualification.from_environment(CONFIG, environment(INPUT_DIR, PACKAGE))
PROFILE_LANE = Qualification.from_environment(CONFIG, environment(INPUT_DIR, PACKAGE, PROFILE))


def test_a_local_run_builds_both_families() -> None:
    artifacts = planned("test-artifacts", LOCAL)
    glowup = planned("test-glowup", LOCAL)

    assert SETTINGS.verify_inputs_script not in artifacts
    assert "assets" in artifacts
    assert SETTINGS.glowup_script not in glowup
    assert "package." in glowup


def test_a_binary_release_verifies_pulled_assets_and_proves_the_pulled_package() -> None:
    artifacts = planned("test-artifacts", BINARY)
    glowup = planned("test-glowup", BINARY)

    assert SETTINGS.verify_inputs_script in artifacts
    # No profile: the binary lane resolves every profile the manifest names,
    # so there is no single one to boot here.
    assert SETTINGS.prove_profile_assets_script not in artifacts
    assert SETTINGS.glowup_script in glowup
    assert "package." not in glowup, "a release lane must not rebuild the package it was handed"


def test_a_profile_release_boots_the_one_profile_it_is_publishing() -> None:
    artifacts = planned("test-artifacts", PROFILE_LANE)

    assert SETTINGS.verify_inputs_script in artifacts
    assert SETTINGS.prove_profile_assets_script in artifacts
    assert VALUES[PROFILE] in artifacts


def test_the_functional_module_signs_locally_and_never_in_a_release_lane() -> None:
    assert "sign" in planned("test-functional", LOCAL)
    assert "sign" not in planned("test-functional", BINARY)
    assert "sign" not in planned("test-functional", PROFILE_LANE)


# ---------------------------------------------------------------------------
# The two hybrids review reproduced, which must now be unrepresentable
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    ("present", "hybrid"),
    (
        ((INPUT_DIR,), "pulled assets proved against a locally rebuilt package"),
        ((PACKAGE,), "a pulled package proved against locally rebuilt assets"),
    ),
)
def test_a_dropped_workflow_line_cannot_build_a_hybrid_plan(
    present: tuple[str, ...], hybrid: str, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Through the command's own construction, not just the parser.

    The refusal has to arrive while the plan is being built. Reached any later
    and the release has already spent the gate proving the wrong bytes.
    """
    from helpers.gate import RecordingRunner

    from capsem.gate import cli  # noqa: F401 - registers every command
    from capsem.gate.command import GateCommand

    for name in VALUES:
        monkeypatch.delenv(name, raising=False)
    for name in present:
        monkeypatch.setenv(name, VALUES[name])

    with pytest.raises(GateError, match="release"):
        GateCommand.registry["candidate"](
            RecordingRunner(PROJECT_ROOT),
            argparse.Namespace(dry_run=False, graph=False, timing=False),
        )._describe()


# ---------------------------------------------------------------------------
# The workflows that have to export a complete state
# ---------------------------------------------------------------------------

WORKFLOWS = PROJECT_ROOT / ".github/workflows"


def exported(workflow: str) -> set[str]:
    """Variable names the workflow writes into `GITHUB_ENV`."""
    text = (WORKFLOWS / workflow).read_text(encoding="utf-8")
    return {name for name in VALUES if f'echo "{name}=' in text}


def test_the_binary_lane_exports_a_complete_binary_release_state() -> None:
    assert exported("release.yaml") == {INPUT_DIR, PACKAGE}


def test_the_profile_lane_exports_a_complete_profile_release_state() -> None:
    assert exported("release-assets.yaml") == {INPUT_DIR, PACKAGE, PROFILE}
