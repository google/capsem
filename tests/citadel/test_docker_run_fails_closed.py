"""Citadel guard: a Dockerfile `RUN` that does several things must stop at the
first failure.

The Citadel is where Capsem records architectural mistakes that must not be
repeated. This one is not yet a mistake in this tree -- there are no violations
-- and it is written now because the surface it guards was unreadable until
there was a parser, and because the failure it prevents is silent.
"""

from __future__ import annotations

from pathlib import Path

from capsem_builder.gate import config as gate_config
from capsem_builder.gate import shellsurfaces
from capsem_builder.gate.shellnodes import AndOr, Command, Pipeline, walk
from capsem_builder.gate.shellparse import parse

ROOT = Path(__file__).resolve().parents[2]

#: `RUN` bodies that sequence deliberately without `set -e`, each with its
#: reason. The shared exclusion shape, so this cannot become a bare list of
#: filenames with one comment above it.
TOLERATED = gate_config.load(ROOT).boundary.sequenced_runs

DOCKER_RATIONALE = """\
`RUN a; b` and `RUN a && b` look alike and are not. Docker runs the body
through `/bin/sh -c`, which does not set `-e`, so the instruction's exit status
is the status of the *last* command in it. Everything before it can fail, and
the layer is committed and cached as though it had worked.

This is not the same rule GitHub Actions needs, and assuming it was would have
filed thirty-eight false reports here: a workflow `run:` body executes under
`bash -e {0}`, so errexit is already on. Same-looking shell, different
contract, and the difference is invisible in the text.

Every multi-command `RUN` in this tree already opens with `set -e` in some
spelling, so this guard starts at zero rather than carrying a debt list. That
is the point of writing it now: the invariant is free to hold today and
expensive to restore once something has silently depended on breaking it.

Read with a parser. `set -eux` is a command with options, `; ` inside a quoted
argument separates nothing, and a heredoc body is data -- three distinctions a
pattern gets wrong in the direction that reports success.

See build_system/builder/gate/shellparse.py and skills/dev-gate/SKILL.md.
"""


def run_bodies() -> dict[str, str]:
    """Every `RUN` body Docker will execute, keyed by where it came from.

    Through the shared extractor, and with the `.j2` templates *rendered*. The
    first version of this read the raw templates, so `{{ arch.kernel_image }}`
    reached the lexer as shell and a correctly chained `make && ls` came back
    as two statements. `shellsurfaces` renders them for exactly this reason:
    the rendered output is what runs.
    """
    return shellsurfaces.dockerfile_bodies(
        ROOT / "docker",
        ROOT / "config" / "docker",
        lambda templates: shellsurfaces.rendered_templates(
            templates, ROOT / "config" / "docker" / "image"
        ),
    )


def sets_errexit(nodes) -> bool:
    """Whether the body turns on errexit, in any of its spellings.

    `set -e`, `set -eux`, `set -o errexit`. Read as a command with arguments
    rather than matched, so `echo "set -e"` does not count and `set -u` alone
    does not either.
    """
    for node in walk(nodes):
        if not isinstance(node, Command) or node.program != "set":
            continue
        options = node.argv[1:]
        if any(word.startswith("-") and not word.startswith("--") and "e" in word[1:] for word in options):
            return True
        if "errexit" in options:
            return True
    return False


def statements(nodes) -> list:
    """Top-level statements: what `sh -c` runs one after another.

    Only the last one's status becomes the instruction's. A `&&` chain is one
    statement however long -- it is an `AndOr` node, not a list of commands --
    and that is exactly the form this guard is asking people to use. Leaving
    `AndOr` out of this counted a correctly-chained body as zero statements,
    which passed for the wrong reason.
    """
    return [node for node in nodes if isinstance(node, Command | Pipeline | AndOr)]


def test_a_multi_command_run_stops_at_the_first_failure() -> None:
    bodies = run_bodies()
    assert len(bodies) > 20, "scanned too few RUN bodies to trust this guard"

    excused = {entry.subject for entry in TOLERATED}
    offenders = [
        f"{where} runs "
        f"{[node.program for node in statements(parse(body)) if isinstance(node, Command)][:6]} "
        "in sequence without set -e"
        for where, body in sorted(bodies.items())
        if len(statements(parse(body))) > 1
        and not sets_errexit(parse(body))
        and where not in excused
    ]
    assert not offenders, DOCKER_RATIONALE + "\n" + "\n".join(offenders)


def test_the_guard_reads_the_difference_it_claims_to() -> None:
    """Break it here, so a refactor that blinds it fails rather than passes."""
    assert not sets_errexit(parse("apt-get update; apt-get install -y x"))
    assert sets_errexit(parse("set -eux; apt-get update; apt-get install -y x"))
    assert sets_errexit(parse("set -o errexit\napt-get update"))
    assert not sets_errexit(parse('echo "set -e"; apt-get update')), "a quoted mention is not a set"
    assert not sets_errexit(parse("set -u; apt-get update")), "nounset is not errexit"

    assert len(statements(parse("a && b && c"))) == 1, "a chain is one statement"
    assert len(statements(parse("a; b; c"))) == 3
    assert len(statements(parse("a | b"))) == 1, "a pipeline is one statement"
    assert len(statements(parse('echo "a; b"'))) == 1, "a separator in a string separates nothing"


def test_every_tracked_dockerfile_yields_run_bodies() -> None:
    """An extractor that silently returns nothing reports a clean tree.

    The guard above is vacuous over an empty body list, and that is precisely
    how a parsing regression would present: green, instantly, everywhere.
    """
    bodies = run_bodies()
    assert len(bodies) > 20, f"only {len(bodies)} RUN bodies extracted; the extractor regressed"
    assert all(body.strip() for body in bodies.values()), "an empty RUN body means a bad parse"
