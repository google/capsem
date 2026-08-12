"""The cheap checks, as independent steps rather than backgrounded jobs.

These were seven `&` and seven `wait` in one recipe body, aggregating into a
single `FAIL=1`. That told an operator something broke, and then they read the
interleaved output of seven concurrent jobs to find out what. Every one of them
is independent -- none reads what another writes -- so as graph nodes they run
at once by construction and every failure comes back named.

The web surfaces are here too, and one of them is not independent: `capsem-app`
embeds `frontend/dist` at compile time, so clippy reads a directory the
frontend build produces. The shell expressed that as a conditional which
skipped clippy entirely when the frontend failed -- losing the clippy result on
exactly the runs where the most had changed. It is an edge.
"""

from __future__ import annotations

from . import toolchain
from .actions import Run, Script
from .config import GateConfig
from .execution import Step, step


def all_of(config: GateConfig) -> list[Step]:
    """Every audit, in no particular order because there is none."""
    audits = config.audits
    return [
        # These three query mutable advisory authorities at qualification time.
        # Locked language dependencies can be materialized before the sandbox;
        # current RustSec, npm bulk, and OSV answers cannot.  Keep only these
        # explicit actions on the authenticated scoped-egress runner.
        step("audit.cargo", Script(audits.cargo, outside_sandbox=True)),
        step("audit.pnpm", Script(audits.pnpm, outside_sandbox=True)),
        step(
            "audit.python-lock",
            Run(["bash", audits.python_lock], outside_sandbox=True),
        ),
        step("audit.public-surface", Script(audits.public_surface)),
        step(
            "audit.skills",
            Run(["uv", "run", "capsem-builder", "validate-skills", audits.skills_dir]),
        ),
        step("audit.release-selections", Run(["bash", audits.hardcoded_selections])),
    ]


def source_syntax(config: GateConfig) -> Step:
    """Parse every source file before anything spends time on it."""
    return step("audit.source-syntax", Script(config.audits.source_syntax))


def generated_settings(config: GateConfig) -> Step:
    """Produce the settings schema, defaults and mock the web surfaces import.

    `frontend/src/lib/mock-settings.generated.ts` is gitignored, so it is not
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
    )


def web_surfaces(config: GateConfig) -> list[Step]:
    """One step per surface, so a failure says which one.

    Each claims `astro_build`, so they run one at a time. Astro stages
    prerendering in a path derived from the project root rather than from the
    invocation -- `<outDir>/.prerender/` when the output is inside the root,
    `<root>/.astro/` when it is not -- so neither `--outDir` nor `--cacheDir`
    isolates two concurrent builds; they delete each other's staging.

    The four surfaces do have distinct roots today, so serializing them is
    insurance rather than a fix. It is insurance worth buying: a build is well
    under a second, and the alternative is a rule that holds only as long as
    nobody adds a second consumer of one root.
    """
    surfaces = config.websurfaces
    return [
        step(
            f"web.{target}",
            Run(["bash", surfaces.script, target]),
            contends=(config.exclusive("astro_build"),),
        )
        for target in surfaces.targets
    ]


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
