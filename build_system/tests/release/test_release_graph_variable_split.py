"""One name meant two things, and the ambiguity had grown a branch.

`CAPSEM_RELEASE_CHANNEL_DIST` was read by `loadReleaseData` to decide *what to
render*, and by `overlay-dist.mjs` to decide *where to copy the built output*.
Those are an input and an output, and nothing but convention kept a caller from
setting one meaning where the other was expected.

The overload then grew a branch to survive itself: the overlay inspected the
path and skipped its own work when it turned out to be a file, because a file
meant "graph fixture" -- the *input* meaning -- arriving at the output variable.
A polymorphic check on a path's type, standing in for the distinction the name
should have made.

`CAPSEM_RELEASE_GRAPH` is the input; `CAPSEM_RELEASE_CHANNEL_DIST` is the output
directory and only that. The branch is deleted with the ambiguity that required
it, and no compatibility path is kept: both consumers are in this repository, so
the rename lands atomically.
"""

from __future__ import annotations

from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[3]

INPUT = "CAPSEM_RELEASE_GRAPH"
OUTPUT = "CAPSEM_RELEASE_CHANNEL_DIST"

#: Everything that could name either variable. Generated rather than listed, so
#: a new caller cannot be added without this noticing.
SEARCHED = (
    "build_system/release_site",
    "scripts",
    "src/capsem",
    ".github/workflows",
    "skills",
    "tests",
    "web/docs",
)

SKIP_DIRS = {"node_modules", "dist", ".astro", "__pycache__", ".git"}


def _sources() -> list[Path]:
    found: list[Path] = []
    for root in SEARCHED:
        base = PROJECT_ROOT / root
        if not base.is_dir():
            continue
        for path in base.rglob("*"):
            if not path.is_file() or path.suffix in {".png", ".ico", ".woff2"}:
                continue
            if SKIP_DIRS & set(path.parts):
                continue
            found.append(path)
    assert len(found) > 100, "scanned too few files to trust this guard"
    return found


def _code(path: Path) -> str:
    """A file's source with its comments stripped.

    These assertions are about what the code reads, not about what its
    comments explain -- and the comments here necessarily name both variables
    to say why they were split.
    """
    lines = []
    for line in path.read_text(encoding="utf-8", errors="ignore").splitlines():
        stripped = line.strip()
        if stripped.startswith(("//", "#", "*", "/*")):
            continue
        lines.append(line)
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# The input
# ---------------------------------------------------------------------------


def test_the_renderer_reads_the_input_variable() -> None:
    """`loadReleaseData` chooses what to render, so it reads the graph name."""
    source = _code(PROJECT_ROOT / "build_system/release_site/src/lib/release-data.ts")

    assert f"process.env.{INPUT}" in source


def test_the_renderer_does_not_read_the_output_variable() -> None:
    """The whole point of the split: an output name cannot select an input."""
    source = _code(PROJECT_ROOT / "build_system/release_site/src/lib/release-data.ts")

    assert OUTPUT not in source


# ---------------------------------------------------------------------------
# The output
# ---------------------------------------------------------------------------


def test_the_overlay_no_longer_guesses_from_the_paths_type() -> None:
    """The branch existed only because one name meant two things.

    A *file* meant "this is really a graph fixture, do nothing"; a *directory*
    meant "overlay here". With the names separated the output is always a
    directory, and a file arriving there is a caller's bug rather than a mode.
    """
    source = _code(PROJECT_ROOT / "build_system/release_site/scripts/overlay-dist.mjs")

    assert "statSync(target).isFile()" not in source, (
        "the overlay still branches on whether its target is a file"
    )


def test_the_overlay_reads_only_the_output_variable() -> None:
    source = _code(PROJECT_ROOT / "build_system/release_site/scripts/overlay-dist.mjs")

    assert OUTPUT in source
    assert INPUT not in source


# ---------------------------------------------------------------------------
# No compatibility path
# ---------------------------------------------------------------------------


def test_no_caller_sets_the_output_name_where_a_graph_fixture_goes() -> None:
    """The ambiguity in its most dangerous form.

    A caller with a graph *file* used to set the output variable, because the
    output variable was also the input -- and the overlay grew its file check
    to survive exactly that. Fixtures set the input name now, and a file
    reaching the output is a bug rather than a mode.
    """
    surface = _code(PROJECT_ROOT / "build_system/scripts/web/check-web-surface.sh")

    fixture_lines = [
        line for line in surface.splitlines() if ".json" in line and OUTPUT in line
    ]

    assert not fixture_lines, (
        f"a graph fixture is still handed to the output variable: {fixture_lines}"
    )


@pytest.mark.parametrize(
    ("workflow", "driver", "implementation"),
    [
        (
            "release-binary-staging.yaml",
            "build_system/scripts/release/build-complete-release-channel.py",
            "build_system/builder/release/tools/build_complete_release_channel.py",
        ),
        (
            "release-channel-staging.yaml",
            "build_system/scripts/release/rehearse-asset-channel-staging.sh",
            "build_system/scripts/release/rehearse-asset-channel-staging.sh",
        ),
    ],
)
def test_the_published_workflows_reach_a_driver_that_names_both_roles(
    workflow: str, driver: str, implementation: str
) -> None:
    """One path, two roles -- and now two names.

    For a generated distribution the graph being rendered and the directory
    the render lands in genuinely are the same path, which is why one name
    survived this long. Setting both is what makes that a coincidence rather
    than a contract.

    Both workflows and their implementations are in this repository. Following
    the exact launcher edge keeps a compatibility wrapper from hiding a missing
    role while still allowing the workflow to delegate the whole operation
    instead of copying it.
    """
    workflow_text = (PROJECT_ROOT / ".github/workflows" / workflow).read_text(encoding="utf-8")
    launcher_text = _code(PROJECT_ROOT / driver)
    implementation_text = _code(PROJECT_ROOT / implementation)

    assert driver in workflow_text, f"{workflow} bypasses its shared release-site driver"
    if implementation != driver:
        assert "capsem_builder.release.tools.build_complete_release_channel" in launcher_text
    assert INPUT in implementation_text, f"{implementation} never says which graph to render"
    assert OUTPUT in implementation_text, f"{implementation} never says where the render goes"
    assert "release-site-build" in implementation_text, (
        f"{implementation} does not drive the shared web build"
    )


def test_the_split_is_complete_across_the_repository() -> None:
    """Every file naming either variable names it in exactly one role."""
    both = [
        path
        for path in _sources()
        if path.name != Path(__file__).name
        and INPUT in _code(path)
        and OUTPUT in _code(path)
    ]
    # A caller may name both only if it drives the command that does both
    # halves. `build:channel` is `astro build` (which reads the graph) followed
    # by the overlay (which writes the directory), so for a generated
    # distribution the two are one path -- a coincidence, not a contract.
    #
    # Expressed as the condition rather than as a list of files, so a new
    # caller that names both without running that command is caught, and one
    # that legitimately does is not a maintenance chore.
    drivers = ("build:channel", "release-site-build")
    unexpected = sorted(
        str(path.relative_to(PROJECT_ROOT))
        for path in both
        if not any(driver in _code(path) for driver in drivers)
    )

    assert not unexpected, (
        "these name both variables without driving the command that plays "
        f"both roles: {unexpected}"
    )
