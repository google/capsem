"""Citadel guard: a release lane may only read what it was handed.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one cost five dispatches of a binary release, each about forty
minutes, and every one of them failed for the same reason wearing a different
face.

`qualify-binaries` isolates itself into a private prefix holding tracked files
and nothing else, while the workflow stages the cohort it is meant to prove --
assets, materialized config, the package, the binaries -- into the workspace.
A local `just test` cannot tell the two apart, because there the checkout *is*
the workspace: every path resolves, so a lane that silently read the wrong root
passed here and failed there.

Four separate consumers had it. The profile axis resolved the catalog against
the checkout. The glow-up built `--assets-dir` and `--config-root` from the
checkout layout. The suites were handed no content selection at all and fell
back to the checkout. Each was found by dispatching a release and waiting.

So the rule is checked where it is cheap: the plan is a value, it can be built
without running anything, and these two properties are readable off it.
"""

from __future__ import annotations

from pathlib import Path

from helpers.gate import built_command

from capsem.gate import config as gate_config
from capsem.gate.qualification import from_environment

ROOT = Path(__file__).resolve().parents[2]

#: A staged workspace that is not, and cannot be confused with, the checkout.
STAGED = Path("/staged-release-workspace")


def _release_plan():
    """The binary lane's plan, as a release workflow would build it."""
    config = gate_config.load(ROOT)
    settings = config.modules
    qualification = from_environment(
        config,
        {
            settings.release_input_dir: str(STAGED / "target/candidate-profile-inputs"),
            settings.release_package: str(STAGED / "release-test-package/capsem.deb"),
            settings.release_bin_dir: str(STAGED / "target/debug"),
        },
    )
    command = built_command(
        ROOT,
        "qualify-binaries",
        (("workspace_root", STAGED),),
        qualification,
    )
    return config, command._describe()


def _rendered(plan) -> list[tuple[str, str]]:
    return [(step.label, action.render()) for step in plan.steps for action in step.actions]


def test_no_step_reads_a_staged_input_from_the_checkout() -> None:
    """The cohort lives where the lane staged it, never where the source is.

    Named per input rather than as one blanket ban on the checkout path: the
    run legitimately writes its own scratch under `target/`, and a guard that
    could not tell those apart would have to be switched off.
    """
    config, plan = _release_plan()
    functional = config.functional
    staged_inputs = {
        "assets": ROOT / functional.assets_dir,
        "materialized config": ROOT / functional.config_root,
        "host binary": ROOT / functional.binary,
    }

    offenders = [
        f"{label} reads {name} from the checkout: {rendered}"
        for label, rendered in _rendered(plan)
        for name, path in staged_inputs.items()
        if str(path) in rendered
    ]

    assert not offenders, (
        "a release lane qualifies from a private prefix that carries only "
        "tracked files, so a staged input named under the checkout is a path "
        "nothing ever wrote:\n  " + "\n  ".join(offenders)
    )


def test_every_suite_is_told_which_content_to_prove() -> None:
    """A suite with no content selection does not fail -- it proves the wrong thing.

    `content_assets_root()` falls back to `<checkout>/assets` when the variable
    is absent, so an unselected suite runs against whatever the source tree
    happens to hold. In a prefix that is nothing, which is how this surfaced;
    on a developer machine it is the last local build, which is worse.
    """
    config, plan = _release_plan()
    content = config.environment.content(assets="A", profiles="P")
    variables = tuple(content)

    unselected = [
        label
        for label, rendered in _rendered(plan)
        if "-m pytest" in rendered
        and not all(f"{variable}={STAGED}" in rendered for variable in variables)
    ]

    assert not unselected, (
        "these suites run without being told which assets and profiles to "
        f"prove, so they fall back to the checkout: {sorted(set(unselected))}"
    )


def _pairing_job() -> str:
    """The workflow job that runs `qualify-binaries`, as text."""
    workflow = (ROOT / ".github/workflows/release.yaml").read_text(encoding="utf-8")
    start = workflow.index("  test-binary-pairing:")
    end = workflow.index("\n  create-release:", start)
    return workflow[start:end]


def test_the_lane_only_installs_workspaces_its_job_has_warmed() -> None:
    """`pnpm install` is a network call for anything the store does not hold.

    The lane runs inside a namespace with only loopback, so a workspace the
    job never installed is not a slow no-op -- it is `EAI_AGAIN`, which is how
    a run died after building and signing everything first.
    """
    config = gate_config.load(ROOT)
    job = _pairing_job()

    unwarmed = [
        workspace
        for workspace in config.functional.node_workspaces
        if f"cd {workspace} && pnpm install" not in job
    ]

    assert not unwarmed, (
        "the pairing job never installs these, so the store cannot hold them "
        f"and the lane would reach the registry offline: {unwarmed}"
    )


def test_a_lane_that_compiles_rust_is_given_its_crates() -> None:
    """The generated mock is built by cargo, and cargo cannot fetch here either.

    The suites read a gitignored generated file, so the plan builds it, so the
    plan compiles `capsem-core` -- in a lane that otherwise compiles nothing
    and cannot reach crates.io.
    """
    _, plan = _release_plan()
    compiles = [
        label
        for label, rendered in _rendered(plan)
        if "generate-settings" in rendered or "cargo" in rendered
    ]
    if not compiles:
        return

    assert "cargo fetch --locked" in _pairing_job(), (
        f"these steps compile Rust: {sorted(set(compiles))}, and the pairing "
        "job does not materialize locked dependencies first"
    )


def test_what_a_suite_needs_is_built_before_it_runs() -> None:
    """Both prerequisites are gitignored, so the plan makes them or nothing does.

    Order is asserted rather than assumed: a step that produces `node_modules`
    or the generated mock after the suite that reads them is the same failure
    as not having it, and reads as present in a step list.
    """
    _, plan = _release_plan()
    labels = list(plan.labels)

    def first(predicate) -> int:
        return next((index for index, label in enumerate(labels) if predicate(label)), -1)

    suite = first(lambda label: label.startswith("functional.pytest."))
    assert suite >= 0, "the release lane runs no pytest suite at all"

    for produced, needed in (
        ("functional.toolchain.node", "node_modules the release site imports"),
        ("functional.audit.generated-settings", "the generated settings mock"),
    ):
        index = first(lambda label, produced=produced: label == produced)
        assert 0 <= index < suite, (
            f"{needed} is produced by {produced!r}, which does not run before "
            f"{labels[suite]!r}"
        )


def test_the_fixture_clears_every_variable_that_picks_a_lane() -> None:
    """A test that builds a plan must not inherit the release it runs inside.

    `qualify-binaries` runs the suite with these exported, and two tests asked
    for the candidate plan, were handed the pulled one, and reported the
    missing package build as a defect. The set is asserted against the parser
    rather than restated: a fourth variable that decides a lane has to be
    cleared too, and nothing else would say so.
    """
    # Read as a module: the fixture's list is the subject, not a helper to call.
    import tests.conftest as suite_conftest

    config = gate_config.load(ROOT)
    settings = config.modules
    decides_a_lane = {
        settings.release_input_dir,
        settings.release_package,
        settings.release_profile,
        settings.release_bin_dir,
    }

    assert set(suite_conftest._GATE_QUALIFICATION_VARIABLES) == decides_a_lane


def test_the_install_proof_is_told_which_url_the_package_polls() -> None:
    """`manifest_url` is where an install keeps looking; `checked_url` is where
    these bytes came from. Comparing the first against the candidate file URL
    failed three install jobs that had installed the package correctly.
    """
    workflow = (ROOT / ".github/workflows/release.yaml").read_text(encoding="utf-8")
    proofs = [
        block
        for block in workflow.split("verify-installed-release.py")[1:]
        if "PREACTIVATION_MANIFEST" in block.split("python3")[0]
    ]
    assert proofs, "no install proof runs against a pre-activation manifest"

    missing = [block for block in proofs if "--metadata-manifest-url" not in block.split("\n\n")[0]]
    assert not missing, (
        "an install proof compares the installed polling URL against the "
        "candidate it was pointed at, which no correct install can satisfy"
    )


def test_the_binaries_are_staged_where_the_lane_is_told_to_find_them() -> None:
    """Two lines, one directory, and nothing checked that they agree.

    The job stages the candidate binaries with `--binary-dir` and separately
    exports the directory the qualification reads. They are written apart, so
    they can drift apart, and the failure would be a glow-up installing from
    an empty directory rather than a missing-file error anyone could read.
    """
    job = _pairing_job()
    staged = [
        line.split("--binary-dir", 1)[1].strip().strip("\\").strip()
        for line in job.splitlines()
        if "--binary-dir" in line
    ]
    exported = [
        line.split("=", 1)[1].strip().strip('"')
        for line in job.splitlines()
        if "CAPSEM_RELEASE_BIN_DIR=" in line
    ]
    assert staged and exported, f"staged={staged} exported={exported}"

    for directory in exported:
        relative = directory.removeprefix("$PWD/")
        assert relative in staged, (
            f"the lane reads binaries from {directory!r} and the job stages "
            f"them into {staged!r}"
        )


def test_only_the_rehearsal_may_build_a_release_state_in_code() -> None:
    """A pulled qualification comes from a workflow, or from one named place.

    `from_environment` refuses a half-set environment because a hybrid proof --
    manifest-selected bytes in one artifact family, source-built bytes in the
    other -- is green, takes an hour, and looks exactly like a release. The
    mirror-image risk is a module that constructs the state itself to reach
    some branch it wants, which no environment check can see.

    So `qualification.rehearsal` is the one constructor outside the parser, it
    says in its own docstring what it is for, and this holds the count at one.
    """
    package = ROOT / "src/capsem/gate"
    offenders = [
        f"{path.relative_to(ROOT)}: {line.strip()}"
        for path in sorted(package.glob("*.py"))
        if path.name != "qualification.py"
        for line in path.read_text(encoding="utf-8").splitlines()
        if "BinaryQualification(" in line or "ProfileQualification(" in line
    ]
    assert not offenders, (
        "these build a release qualification directly instead of going "
        "through `from_environment` or `qualification.rehearsal`:\n  "
        + "\n  ".join(offenders)
    )
