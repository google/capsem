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
from helpers.workflow_contract import workflow_reachable_text

PROJECT_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PROJECT_ROOT / "tests"))

from capsem.gate import config as gate_config  # noqa: E402
from capsem.gate.errors import GateError  # noqa: E402
from capsem.gate.qualification import Mode, Qualification  # noqa: E402
from capsem.gate.qualification import from_environment as qualification_for  # noqa: E402

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
    qualification = qualification_for(CONFIG, environment(*present))

    assert qualification.mode is mode
    assert qualification.pulled is (mode is not Mode.LOCAL)


@pytest.mark.parametrize("present", PARTIAL)
def test_every_partial_release_environment_is_refused(present: tuple[str, ...]) -> None:
    """Naming both sides, because the operator has to fix the missing one."""
    with pytest.raises(GateError) as raised:
        qualification_for(CONFIG, environment(*present))

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
    assert qualification_for(CONFIG, {INPUT_DIR: "", PACKAGE: ""}).mode is Mode.LOCAL

    with pytest.raises(GateError):
        qualification_for(CONFIG, {INPUT_DIR: VALUES[INPUT_DIR], PACKAGE: "   "})


def test_the_profile_release_carries_its_profile_and_the_binary_release_does_not() -> None:
    binary = qualification_for(CONFIG, environment(INPUT_DIR, PACKAGE))
    profile = qualification_for(CONFIG, environment(INPUT_DIR, PACKAGE, PROFILE))

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


LOCAL = qualification_for(CONFIG, {})
BINARY = qualification_for(CONFIG, environment(INPUT_DIR, PACKAGE))
PROFILE_LANE = qualification_for(CONFIG, environment(INPUT_DIR, PACKAGE, PROFILE))


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


def test_a_deferred_profile_proves_assets_without_inventing_a_package() -> None:
    """A cold channel has no package, but its immutable profile still boots.

    This is deliberately a separate private command rather than a fourth
    release qualification shape: no functional or glow-up module may consume
    a package-less pairing, and the complete three-state release union stays
    fail-closed.
    """
    from helpers.gate import RecordingRunner

    from capsem.gate import cli
    from capsem.gate.command import GateCommand

    assert cli.COMMAND_MODULES  # importing the CLI registers every command

    command = GateCommand.registry["test-profile-artifacts"](
        RecordingRunner(PROJECT_ROOT),
        argparse.Namespace(
            dry_run=False,
            graph=False,
            timing=False,
            input_dir=VALUES[INPUT_DIR],
            profile=VALUES[PROFILE],
        ),
    )
    rendered = command._describe().describe()

    assert command.uses_qualification is False
    assert SETTINGS.verify_inputs_script in rendered
    assert SETTINGS.prove_profile_assets_script in rendered
    assert VALUES[INPUT_DIR] in rendered
    assert VALUES[PROFILE] in rendered
    assert VALUES[PACKAGE] not in rendered
    for forbidden in ("build-assets", "_build-kernel", "_build-rootfs", "test-functional"):
        assert forbidden not in rendered


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
    """Variable names the workflow or its direct script writes to `GITHUB_ENV`."""
    text = workflow_reachable_text(PROJECT_ROOT, WORKFLOWS / workflow)
    return {name for name in VALUES if f'echo "{name}=' in text}


def test_the_binary_lane_exports_a_complete_binary_release_state() -> None:
    assert exported("release.yaml") == {INPUT_DIR, PACKAGE}


def test_the_profile_lane_exports_a_complete_profile_release_state() -> None:
    assert exported("release-assets.yaml") == {INPUT_DIR, PACKAGE, PROFILE}


# ---------------------------------------------------------------------------
# Diagnostics stay available exactly when the environment is broken
# ---------------------------------------------------------------------------
#
# A half-exported release environment is the moment an operator most needs to
# ask what the last run did. Parsing the release state in every command's
# constructor made `runs last` and `gc --dry-run` refuse with the same message
# as the gate itself -- correct for a command that would *prove* something,
# useless for one that only reports.

#: Commands that only read. None of them can prove anything, so none of them
#: has an opinion about which artifacts a release selected.
INSPECTION = (
    ("runs", {"action": "last", "failed": False, "run": None}),
    ("gc", {"dry_run": True}),
    ("version", {}),
    ("logs", {"target": "service"}),
)


def _construct(name: str, args: dict):
    from helpers.gate import RecordingRunner

    from capsem.gate import cli  # noqa: F401 - registers every command
    from capsem.gate.command import GateCommand

    namespace = argparse.Namespace(dry_run=False, graph=False, timing=False)
    for key, value in args.items():
        setattr(namespace, key, value)
    return GateCommand.registry[name](RecordingRunner(PROJECT_ROOT), namespace)


@pytest.mark.parametrize(("name", "args"), INSPECTION, ids=[n for n, _ in INSPECTION])
def test_an_inspection_command_survives_a_partial_release_environment(
    name: str, args: dict, monkeypatch: pytest.MonkeyPatch
) -> None:
    """`runs last` is what you reach for *because* the workflow broke."""
    for variable in VALUES:
        monkeypatch.delenv(variable, raising=False)
    monkeypatch.setenv(PACKAGE, VALUES[PACKAGE])

    command = _construct(name, args)
    command._describe()  # it can still plan; that is the whole point

    assert command.uses_qualification is False


@pytest.mark.parametrize(
    ("name", "args"),
    (
        ("candidate", {}),
        ("test-artifacts", {}),
        ("test-functional", {}),
        ("test-glowup", {}),
        ("release-binaries", {"channel": "stable"}),
        ("release-profile", {"channel": "stable", "profile": "code"}),
    ),
)
def test_a_qualifying_command_still_refuses_a_partial_environment(
    name: str, args: dict, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The refusal has to survive being made selective."""
    for variable in VALUES:
        monkeypatch.delenv(variable, raising=False)
    monkeypatch.setenv(PACKAGE, VALUES[PACKAGE])

    with pytest.raises(GateError, match="release"):
        _construct(name, args)


def test_the_capability_is_declared_rather_than_guessed() -> None:
    """Whether a command proves artifacts is a property of the command.

    Inferring it -- from the name, from whether the plan happens to mention a
    release script -- puts the answer somewhere nobody looks when adding the
    next module.
    """
    from capsem.gate import cli  # noqa: F401 - registers every command
    from capsem.gate.command import GateCommand

    qualifying = {name for name, cls in GateCommand.registry.items() if cls.uses_qualification}

    assert qualifying == {
        "candidate",
        "test-candidate",
        "test-artifacts",
        "test-functional",
        "test-glowup",
        # The per-lane verbs CI calls, which compose the three modules above.
        "qualify-assets",
        "qualify-binaries",
        "release-binaries",
        "release-profile",
    }


# ---------------------------------------------------------------------------
# The states are unrepresentable, not merely unreachable through one parser
# ---------------------------------------------------------------------------


def test_a_local_state_cannot_carry_release_inputs() -> None:
    """`from_environment` refuses partials; direct construction did not.

    A dataclass with four optional fields makes `LOCAL` with an input
    directory a perfectly ordinary object, so the invariant lived in one
    function rather than in the type.
    """
    import pydantic

    from capsem.gate.qualification import LocalQualification

    # Through a named mapping, so the type checker does not report the very
    # error this asserts happens at *runtime*. It would be right -- that is
    # the point -- and a literal splat is what ruff objects to instead.
    local_with_release_input = {"bin_dir": "target/debug", "input_dir": VALUES[INPUT_DIR]}

    with pytest.raises((pydantic.ValidationError, TypeError)):
        LocalQualification(**local_with_release_input)


def test_a_binary_release_cannot_be_built_without_its_package() -> None:
    import pydantic

    from capsem.gate.qualification import BinaryQualification

    without_package = {"input_dir": VALUES[INPUT_DIR], "bin_dir": "target/debug"}

    with pytest.raises(pydantic.ValidationError):
        BinaryQualification(**without_package)


def test_a_release_path_may_not_be_empty_or_whitespace() -> None:
    """`GatePath` is about the text, not the filesystem.

    Existence is a plan action; a `--dry-run` that stats the disk is a dry run
    that depends on the machine it is describing.
    """
    import pydantic

    from capsem.gate.qualification import BinaryQualification

    with pytest.raises(pydantic.ValidationError):
        BinaryQualification(input_dir="   ", package=VALUES[PACKAGE], bin_dir="target/debug")


def test_a_profile_name_follows_the_configured_grammar() -> None:
    import pydantic

    from capsem.gate.qualification import ProfileQualification

    with pytest.raises(pydantic.ValidationError):
        ProfileQualification(
            input_dir=VALUES[INPUT_DIR],
            package=VALUES[PACKAGE],
            bin_dir="target/debug",
            profile="not a profile name",
        )
