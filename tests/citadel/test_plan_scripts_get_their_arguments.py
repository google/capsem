"""Citadel guard: a plan step must be able to start the script it names.

`glowup.package` and `glowup.channel-switch` are the last two steps of a binary
release, and they ran `scripts/local-release-glowup.py` without `--source-commit`,
`--evidence-dir` or `--profile-revision-policy`. All three were `required=True`.
Neither step could ever have got past `argparse`.

Nothing found it, and the reason is worth recording. The script is exercised
constantly -- `installproof.prove_glowup` runs it on every local `just test-clean`,
passing all three -- so no test of the script was failing, no import was
broken, and the call site had no test of its own because the plan is a
description and descriptions look fine. Only the release lane reaches these two
steps, and reaching them costs a forty-minute dispatch.

So the arguments are checked where they are cheap: a plan is a value, and the
script it names is a file on disk that declares what it accepts. Both halves
are readable without running either.

Structural rather than by importing the parser: these scripts build their
`ArgumentParser` inside `main()`, so getting the real object means running the
program. `ast` reads the same `add_argument` calls the parser would.
"""

from __future__ import annotations

import ast
from pathlib import Path

from capsem_builder.gate import config as gate_config
from capsem_builder.gate.qualification import from_environment
from helpers.gate import PROJECT_ROOT, built_command

ROOT = Path(__file__).resolve().parents[2]

#: A staged workspace that is not, and cannot be confused with, the checkout.
STAGED = Path("/staged-release-workspace")


def _lanes():
    """Every command whose plan differs by lane, in each shape it has.

    The local lane is what `just test-clean` builds; the two release shapes are what
    the workflows build. A guard that only checked the local one would have
    passed on the defect this file exists for.
    """
    config = gate_config.load(ROOT)
    settings = config.modules
    release = {
        settings.release_input_dir: str(STAGED / "target/candidate-profile-inputs"),
        settings.release_package: str(STAGED / "release-test-package/capsem.deb"),
        settings.release_bin_dir: str(STAGED / "target/debug"),
    }
    binary = from_environment(config, release)
    profile = from_environment(config, {**release, settings.release_profile: "code"})
    return (
        ("candidate", (), from_environment(config, {})),
        ("qualify-binaries", (("workspace_root", STAGED),), binary),
        (
            "qualify-assets",
            (
                ("input_dir", STAGED / "target/candidate-profile-inputs"),
                ("profile", "code"),
                ("workspace_root", STAGED),
                ("activation_ready", "true"),
            ),
            profile,
        ),
    )


def _script_invocations() -> list[tuple[str, str, list[str]]]:
    """Every `uv run python <checked-in script>` any lane's plan renders."""
    found: list[tuple[str, str, list[str]]] = []
    for name, arguments, qualification in _lanes():
        plan = built_command(ROOT, name, arguments, qualification)._describe()
        for step in plan.steps:
            for action in step.actions:
                tokens = action.render().split()
                if "python" not in tokens:
                    continue
                script = tokens[tokens.index("python") + 1]
                if (ROOT / script).is_file() and script.endswith(".py"):
                    found.append((step.label, script, tokens[tokens.index("python") + 2 :]))
    assert found, "no plan renders a script, so this guard is watching nothing"
    return found


def _declared(script: Path) -> tuple[set[str], set[str]]:
    """The options a script accepts, and the ones it cannot start without.

    Required means declared `required=True` with no default. A default makes
    an option omissible whatever the flag says, and one of the three arguments
    fixed here was made optional-with-a-fallback rather than passed -- which
    this has to treat as satisfied or it would report its own fix.
    """
    tree = ast.parse(script.read_text(encoding="utf-8"))
    accepted: set[str] = set()
    required: set[str] = set()
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call) or not isinstance(node.func, ast.Attribute):
            continue
        if node.func.attr != "add_argument":
            continue
        options = [
            argument.value
            for argument in node.args
            if isinstance(argument, ast.Constant)
            and isinstance(argument.value, str)
            and argument.value.startswith("-")
        ]
        if not options:
            continue
        accepted.update(options)
        keywords = {keyword.arg: keyword.value for keyword in node.keywords}
        demanded = keywords.get("required")
        if (
            isinstance(demanded, ast.Constant)
            and demanded.value is True
            and "default" not in keywords
        ):
            required.update(options[:1])
    return accepted, required


def test_no_step_omits_an_argument_its_script_cannot_start_without() -> None:
    """The defect itself: three required options, none of them passed."""
    missing: list[str] = []
    for label, script, arguments in _script_invocations():
        accepted, required = _declared(ROOT / script)
        if not accepted:
            continue
        absent = sorted(option for option in required if option not in arguments)
        if absent:
            missing.append(f"{label} runs {script} without {', '.join(absent)}")

    assert not missing, (
        "these steps exit on an argparse usage error before doing any work, "
        "and only a release dispatch would ever find out:\n  " + "\n  ".join(missing)
    )


def test_no_step_passes_an_option_its_script_never_declared() -> None:
    """The other direction, which is the same defect after a rename.

    argparse accepts an unknown option no more gracefully than it tolerates a
    missing one, and a flag that quietly stops existing is how a step starts
    proving nothing rather than failing.
    """
    unknown: list[str] = []
    for label, script, arguments in _script_invocations():
        accepted, _ = _declared(ROOT / script)
        if not accepted:
            continue
        for argument in arguments:
            option = argument.split("=", 1)[0]
            if option.startswith("--") and option not in accepted:
                unknown.append(f"{label} passes {option} to {script}, which does not accept it")

    assert not unknown, "\n  ".join(["", *unknown])


def test_the_guard_reads_the_scripts_this_repository_actually_ships() -> None:
    """A path that stops resolving turns both guards above into no-ops.

    They skip anything they cannot find, deliberately -- a rendered command may
    name a script in another checkout -- and that tolerance is exactly what
    would let a moved file silence them.
    """
    scripts = {script for _, script, _ in _script_invocations()}
    assert len(scripts) >= 5, f"only {sorted(scripts)} were checked; the plans render more"
    assert PROJECT_ROOT == ROOT
