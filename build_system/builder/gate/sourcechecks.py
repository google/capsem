"""Ruff and Ty, as three steps a graph can schedule, time and name.

Both lived behind one opaque `Call` delegating to a sequential function that
ran them in order and collected failures into a list by hand. It worked -- and
it did by hand, once, in a place the graph could not see, exactly what the plan
does for every other independent pair of steps.

What that cost: the three checks could not be timed apart, so "the source gate
took four minutes" covered Ruff, strict Ty and relaxed Ty together; a Ruff
failure and a Ty failure were one line in one `GateError`; and neither could
overlap the other when the machine had room.

The split that matters is strict against relaxed. `src/` passes every rule and
is checked with nothing held back. The other trees hold back `ty_ratchet` --
roughly four hundred diagnostics dominated by inference over untyped fixture
data -- because the alternative was checking them loosely or not at all, and
not at all is what actually happened for years.
"""

from __future__ import annotations

from .actions import Run
from .config import GateConfig
from .execution import Kind, Speed, Step
from .plan import Plan


def fragment(plan: Plan, config: GateConfig, *, after: tuple[Step, ...] = ()) -> tuple[Step, ...]:
    """Every Python source check, as independent steps. Returns all of them.

    All the leaves, not the last one: they do not depend on each other, and a
    caller that waited only for whichever was written last would let the next
    phase start while the other two were still going.
    """
    phase = plan.phase("python")
    settings = config.lint
    present = [name for name in settings.python_roots if (config.root / name).is_dir()]
    strict = [name for name in present if name in settings.strict_roots]
    relaxed = [name for name in present if name not in settings.strict_roots]

    steps = [phase.add(_ruff(config), after=after)]
    if strict:
        # No `--ignore`: that is what strict means, and a held-back rule here
        # would be the ratchet quietly growing a second home.
        steps.append(phase.add(_ty("strict", config, strict, ()), after=after))
    if relaxed:
        steps.append(
            phase.add(_ty("relaxed", config, relaxed, tuple(settings.ty_ratchet)), after=after)
        )
    return tuple(steps)


def ruff_argv(config: GateConfig) -> list[str]:
    """Ruff over the whole tree; its own configuration selects the rules."""
    project = config.suites.pytest.build_system_project
    return [
        "uv",
        "run",
        "--project",
        project,
        "--frozen",
        "ruff",
        "check",
        "--config",
        config.suites.pytest.project_manifest,
        ".",
    ]


def ty_argv(
    config: GateConfig, roots: tuple[str, ...] | list[str], *, held_back: tuple[str, ...] = ()
) -> list[str]:
    """One spelling of a `ty` invocation.

    Exported because the guards ask the same question -- does `src` still pass
    with nothing held back, does each ratchet rule still fire -- and a second
    hand-assembled argv is a second thing to keep in step with the first.
    """
    project = config.suites.pytest.build_system_project
    flags = ["--error-on-warning"] if config.lint.error_on_warning else []
    platform = ["--python-platform", config.lint.python_platform]
    ignores = [flag for rule in held_back for flag in ("--ignore", rule)]
    return [
        "uv",
        "run",
        "--project",
        project,
        "--frozen",
        "ty",
        "check",
        "--project",
        project,
        *flags,
        *platform,
        *roots,
        *ignores,
    ]


def ty_inventory_argv(
    config: GateConfig, roots: tuple[str, ...] | list[str]
) -> list[str]:
    """Emit every relaxed-tree diagnostic in a stable, countable format.

    ``--exit-zero`` is intentional: the contract compares the complete output
    against the typed debt baseline instead of treating known diagnostics as a
    process failure. Ty forbids combining it with ``--error-on-warning``;
    warnings are still printed and therefore counted.
    """
    return [
        "uv",
        "run",
        "--project",
        config.suites.pytest.build_system_project,
        "--frozen",
        "ty",
        "check",
        "--project",
        config.suites.pytest.build_system_project,
        "--exit-zero",
        "--output-format",
        "concise",
        "--color",
        "never",
        "--python-platform",
        config.lint.python_platform,
        *roots,
    ]


def _ruff(config: GateConfig):
    from .execution import step

    return step("ruff", Run(ruff_argv(config)), kind=Kind.LINT, speed=Speed.FAST)


def _ty(label: str, config: GateConfig, roots: list[str], held_back: tuple[str, ...]):
    from .execution import step

    return step(
        f"ty.{label}",
        Run(ty_argv(config, roots, held_back=held_back)),
        kind=Kind.LINT,
        speed=Speed.FAST,
    )
