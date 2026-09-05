"""The cheap checks, as independent steps rather than backgrounded jobs.

These were seven background jobs aggregated into one `FAIL=1`, leaving an
operator to untangle interleaved output. Each is independent, so graph nodes
run them concurrently and report every failure by name.

The web surfaces are here too, and one of them is not independent: `capsem-app`
embeds `web/app/dist` at compile time, so clippy reads a directory the
frontend build produces. The shell expressed that as a conditional which
skipped clippy entirely when the frontend failed -- losing the clippy result on
exactly the runs where the most had changed. It is an edge.
"""

from __future__ import annotations

from dataclasses import dataclass

from .actions import Run, Script
from .config import GateConfig
from .execution import Kind, Needs, Speed, Step, step


def all_of(config: GateConfig) -> list[Step]:
    """Every source audit independent of live advisory ordering."""
    audits = config.audits
    project = config.suites.pytest.build_system_project
    return [
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


@dataclass(frozen=True)
class LiveAudits:
    """Ordered live proof: broad OSV coverage before strict Rust policy."""

    dependencies: Step
    rust_policy: Step


def live(config: GateConfig) -> LiveAudits:
    """Return the live dependency proofs in their required order."""
    audits = config.audits
    return LiveAudits(
        dependencies=step(
            "audit.dependencies",
            Script(config, audits.dependencies, outside_sandbox=True),
            kind=Kind.STATIC_TEST,
            needs=frozenset({Needs.NETWORK}),
            speed=Speed.FAST,
        ),
        rust_policy=step(
            "audit.cargo",
            Script(config, audits.cargo, outside_sandbox=True),
            kind=Kind.STATIC_TEST,
            needs=frozenset({Needs.NETWORK}),
            speed=Speed.FAST,
        ),
    )


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


def rust_format(config: GateConfig) -> Step:
    """Reject Rust formatting drift before any workspace compilation."""
    return step(
        "rust-format",
        Run(list(config.modules.rust_format)),
        kind=Kind.LINT,
        speed=Speed.FAST,
    )
