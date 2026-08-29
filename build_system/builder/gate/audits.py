"""The cheap checks, as independent steps rather than backgrounded jobs.

These were seven `&` and seven `wait` in one recipe body, aggregating into a
single `FAIL=1`. That told an operator something broke, and then they read the
interleaved output of seven concurrent jobs to find out what. Every one of them
is independent -- none reads what another writes -- so as graph nodes they run
at once by construction and every failure comes back named.

The web surfaces are here too, and one of them is not independent: `capsem-app`
embeds `web/app/dist` at compile time, so clippy reads a directory the
frontend build produces. The shell expressed that as a conditional which
skipped clippy entirely when the frontend failed -- losing the clippy result on
exactly the runs where the most had changed. It is an edge.
"""

from __future__ import annotations

from . import toolchain
from .actions import Run, Script
from .config import GateConfig
from .execution import SATURATES, Kind, Needs, Speed, Step, step


def all_of(config: GateConfig) -> list[Step]:
    """Every audit, in no particular order because there is none."""
    audits = config.audits
    project = config.suites.pytest.build_system_project
    return [
        *live(config),
        # Sandboxed, and deliberately: it reads `cargo metadata --locked`,
        # resolving from the materialized cache and compiling nothing.
        step(
            "audit.dependency-drift",
            Script(config, audits.dependency_drift),
            kind=Kind.STATIC_TEST,
            speed=Speed.FAST,
        ),
        step(
            "audit.public-surface",
            Script(config, audits.public_surface),
            kind=Kind.STATIC_TEST,
            speed=Speed.FAST,
        ),
        step(
            "audit.skills",
            Run(
                [
                    "uv",
                    "run",
                    "--project",
                    project,
                    "--frozen",
                    "capsem-builder",
                    "validate-skills",
                    audits.skills_dir,
                ]
            ),
            kind=Kind.STATIC_TEST,
            speed=Speed.FAST,
        ),
        step(
            "audit.release-selections",
            Run(["bash", audits.hardcoded_selections]),
            kind=Kind.STATIC_TEST,
            speed=Speed.FAST,
        ),
        # Python has Ruff and strict Ty, Rust has Clippy with warnings denied,
        # the web surfaces fail on warnings -- and every line of shell had
        # nothing at all. Four `# shellcheck disable=` directives were already
        # in the tree, written for a linter no lane ran. All three surfaces are
        # checked and each fails closed.
        step(
            "audit.shell",
            Run(_surface(config, "shell", ",".join(audits.shell_ignore))),
            kind=Kind.LINT,
            speed=Speed.FAST,
        ),
        step(
            "audit.docker",
            Run(_surface(config, "dockerfile", ",".join(audits.docker_ignore))),
            kind=Kind.LINT,
            speed=Speed.FAST,
        ),
        step(
            "audit.markdown",
            Run(_surface(config, "markdown", "")),
            kind=Kind.LINT,
            speed=Speed.FAST,
        ),
    ]


def live(config: GateConfig) -> list[Step]:
    """The three audit answers that can change while source stays unchanged."""
    audits = config.audits
    actions = (
        ("audit.cargo", Script(config, audits.cargo, outside_sandbox=True)),
        ("audit.pnpm", Script(config, audits.pnpm, outside_sandbox=True)),
        ("audit.python-lock", Run(["bash", audits.python_lock], outside_sandbox=True)),
    )
    return [
        step(
            label,
            action,
            kind=Kind.STATIC_TEST,
            needs=frozenset({Needs.NETWORK}),
            speed=Speed.FAST,
        )
        for label, action in actions
    ]


def _surface(config: GateConfig, name: str, exclude: str) -> list[str]:
    """One entry point per surface, through the shared Citadel harness."""
    audits = config.audits
    argv = [
        "uv",
        "run",
        "--project",
        config.suites.pytest.build_system_project,
        "--frozen",
        "python",
        audits.surfaces,
        name,
        "--severity",
        audits.shell_severity,
    ]
    return [*argv, "--exclude", exclude] if exclude else argv


def source_syntax(config: GateConfig) -> Step:
    """Parse every source file before anything spends time on it."""
    return step(
        "audit.source-syntax",
        Script(config, config.audits.source_syntax),
        kind=Kind.LINT,
        speed=Speed.FAST,
    )


def generated_settings(config: GateConfig) -> Step:
    """Produce the settings schema, defaults and mock the web surfaces import.

    `web/app/src/lib/mock-settings.generated.ts` is gitignored, so it is not
    part of the source the gate copies or digests -- it has to be *made*. It
    was not: the fast and static modules ran `_check-generated-settings`, which
    only asserts the committed schema and the generated output agree, and the
    file itself arrived from whatever earlier build happened to leave it.

    On a warm machine that is invisible. On a fresh clone, in CI, or in a
    private copy of the checkout, `svelte-check` stops at `Cannot find module
    './mock-settings.generated'` -- which is how this was found, on the first
    real run from a prefix.

    Before the surfaces and after the Rust toolchain, because the script needs
    `cargo run -p capsem-core --bin mcp_export`. That cost is not new work in
    this lane: clippy builds the same workspace a few steps later.

    Which is also why it claims `workspace_binaries`. It shares that target
    directory with `web.release-channel`, the two can overlap, and cargo's own
    lock would serialise them anyway -- as execution time, inside a step, where
    no instrument the gate has can see it. Declared, the wait is measured.
    """
    return step(
        "audit.generated-settings",
        # The tracked pair goes to scratch. This step wants the mock; it was
        # also rewriting `config/settings/*.generated.json` in the checkout it
        # is qualifying, which `source.verify` tolerated only because the bytes
        # matched and which a sandboxed run cannot do at all.
        Run(
            [
                "bash",
                config.devloop.generate_settings,
                str(config.path(config.devloop.generated_settings_scratch)),
            ]
        ),
        contends=(config.exclusive("workspace_binaries"),),
        kind=Kind.COMPILE,
        speed=Speed.FAST,
    )


def web_surfaces(config: GateConfig) -> list[Step]:
    """One step per surface, so a failure says which one.

    The surfaces that bundle claim `astro_build`, so those run one at a time.
    Astro stages prerendering in a path derived from the project root rather
    than from the invocation -- `<outDir>/.prerender/` when the output is
    inside the root, `<root>/.astro/` when it is not -- so neither `--outDir`
    nor `--cacheDir` isolates two concurrent builds; they delete each other's
    staging.

    The bundling surfaces do have distinct roots today, so serializing them is
    insurance rather than a fix, and the alternative is a rule that holds only
    as long as nobody adds a second consumer of one root.

    Which surfaces those are is read from `building` rather than assumed of all
    of them, and that is the correction this comment used to need. It claimed a
    build was well under a second; `release-site` then spent two minutes in
    `cargo`, holding the Astro exclusive across a Rust build while
    `web.frontend-build` -- the surface on the critical path, because it gates
    clippy -- waited behind it. The Rust half is now `web.release-channel`, and
    what remains here type-checks and runs vitest without bundling anything, so
    it no longer takes the claim at all.
    """
    surfaces = config.websurfaces
    return [
        step(
            f"web.{target}",
            Run(["bash", surfaces.script, target]),
            contends=(config.exclusive("astro_build"),) if target in surfaces.building else (),
            kind=Kind.COMPILE if target in surfaces.building else Kind.STATIC_TEST,
            speed=Speed.FAST,
        )
        for target in surfaces.targets
    ]


def frontend_bundle(config: GateConfig) -> Step:
    """Build the exact bundle Tauri embeds, without rerunning frontend tests."""
    frontend = config.frontend
    return step(
        "web.frontend-bundle",
        Run(["bash", frontend.build_script, frontend.build_target]),
        contends=(
            config.exclusive("astro_build"),
            config.exclusive("node_modules"),
        ),
        kind=Kind.COMPILE,
        speed=Speed.FAST,
    )


def clippy(config: GateConfig) -> Step:
    """The Rust lint gate.

    Clippy rather than `cargo check`: it is a strict superset and covers
    `--all-targets`, which is the project standard. Warnings are errors here
    because the workspace sets `warnings = "deny"` and a gate that let one
    through would be disagreeing with the build.
    """
    return step(
        "clippy",
        Run(
            ["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"],
            env=toolchain.ort_environment(config, toolchain.OrtConsumer.FAST),
        ),
        contends=(config.exclusive("workspace_binaries"),),
        kind=Kind.COMPILE,
        speed=Speed.FAST,
        concurrency=SATURATES,
    )


def release_channel(config: GateConfig) -> Step:
    """Build a release channel twice and prove the two agree.

    Lived inside `web.release-site` until the graph was asked what that step
    cost. It is not a web surface: `pnpm check` and the vitest run take about a
    second between them, and the remaining two minutes are
    `cargo run -p capsem-admin` building a binary the fast phase does not
    otherwise build. The name said "web", the timing report showed one opaque
    line, and one action can hide a compiler.

    Split out so it stops holding `astro_build` -- the claim exists to keep two
    Astro builds from deleting each other's staging, and this held it across a
    Rust build while `web.frontend`, the surface that gates clippy, waited a
    minute for it.

    It takes `workspace_binaries` instead, which is the claim it should have
    had all along: `audit.generated-settings` drives cargo against the same
    target directory and the two are unordered.
    """
    return step(
        "web.release-channel",
        Run(["bash", config.websurfaces.script, "release-channel"]),
        contends=(config.exclusive("workspace_binaries"),),
        kind=Kind.E2E,
        needs=frozenset({Needs.DISK}),
        speed=Speed.SLOW,
    )


def blocking_surface(config: GateConfig, surfaces: list[Step]) -> Step:
    """The surface clippy has to wait for.

    Looked up by the name in config rather than by position, so reordering the
    target list cannot silently move the dependency onto the wrong one.
    """
    wanted = f"web.{config.websurfaces.blocks_clippy}"
    # Suffix, not equality: composed into a larger plan these labels carry
    # their phase's namespace, and matching the whole thing would silently
    # find nothing.
    return next(candidate for candidate in surfaces if candidate.label.endswith(wanted))
