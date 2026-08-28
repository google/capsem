"""Citadel infrastructure: the shell tokenizer's grammar, case by case.

Each case is a line of the grammar in `helpers/shelltokens.py`. The three marked
REGRESSION are bugs this tokenizer actually had, kept as cases so a future
rewrite has to answer them.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
from capsem_builder.gate import shellsurfaces
from helpers.script_modules import load_script
from helpers.shelltokens import UnterminatedQuote, tokenize
from helpers.workflow_contract import workflow_reachable_text

lint_harness = load_script(
    "citadel_shell_lint_harness",
    Path(__file__).resolve().parents[2] / "scripts" / "lint_harness.py",
)

CASES: tuple[tuple[str, str, tuple[tuple[str, ...], ...]], ...] = (
    ("bare words", "test $X = success", (("test", "$X", "=", "success"),)),
    ("double quotes", 'test "$X" = success', (("test", "$X", "=", "success"),)),
    ("single quotes keep content literal", "echo 'a $B c'", (("echo", "a $B c"),)),
    ("empty string is a word", 'echo ""', (("echo", ""),)),
    ("operators are their own tokens", "a && b || c", (("a", "&&", "b", "||", "c"),)),
    ("longest operator wins", "a && b", (("a", "&&", "b"),)),
    ("semicolon separates", "a ; b", (("a", ";", "b"),)),
    ("pipe is an operator", "a | b", (("a", "|", "b"),)),
    ("comment at word start is dropped", "echo hi # note", (("echo", "hi"),)),
    ("hash inside a word is not a comment", 'echo "a#b"', (("echo", "a#b"),)),
    ("whole-line comment yields nothing", "# nothing here", ()),
    ("line continuation joins", "echo a \\\n  b", (("echo", "a", "b"),)),
    ("newline separates logical lines", "a\nb", (("a",), ("b",))),
    ("blank lines are skipped", "a\n\n\nb", (("a",), ("b",))),
    ("backslash escapes outside quotes", r"echo a\ b", (("echo", "a b"),)),
    ("escape inside double quotes", r'echo "a\"b"', (("echo", 'a"b'),)),
    # REGRESSION: lexing line by line raised ValueError here, and accumulating
    # until the quotes balanced produced one blob and lost the command after it.
    (
        "REGRESSION: a word may contain a newline",
        'ROWS="| a |\n"\ntest "$R" = success\n',
        (("ROWS=| a |\n",), ("test", "$R", "=", "success")),
    ),
    ("pipe inside quotes is not an operator", 'echo "a | b"', (("echo", "a | b"),)),
    # REGRESSION: `&` is an operator character, so 2>&1 was split into three
    # tokens and read as a background command.
    ("REGRESSION: 2>&1 is one redirection", "cmd 2>&1", (("cmd", "2>&1"),)),
    ("descriptor binds to the redirect", "cmd 2>/dev/null", (("cmd", "2>", "/dev/null"),)),
    ("append redirect", "cmd >>log", (("cmd", ">>", "log"),)),
    ("here-string", 'cmd <<<"x"', (("cmd", "<<<", "x"),)),
    ("redirect then pipe", "cmd 2>&1 | tee f", (("cmd", "2>&1", "|", "tee", "f"),)),
    # REGRESSION: `$( )` is its own quoting context. The scanner closed the
    # outer double quote on the first quote inside a sed script, which only
    # showed up when the corpus grew to include Dockerfile RUN bodies.
    (
        "REGRESSION: quotes inside command substitution",
        """x="$(sed -n 's/a "b" c/d/p' f)" """,
        (("x=$(sed -n 's/a \"b\" c/d/p' f)",),),
    ),
    ("nested command substitution", "x=$(a $(b) c)", (("x=$(a $(b) c)",),)),
    (
        "github expression is one token",
        'X="${{ needs.a.result }}"',
        (("X=${{ needs.a.result }}",),),
    ),
    (
        "expression whitespace is canonicalized",
        "X=${{   needs.a.result   }}",
        (("X=${{ needs.a.result }}",),),
    ),
)


@pytest.mark.parametrize(
    ("script", "expected"),
    [(script, expected) for _name, script, expected in CASES],
    ids=[name for name, _script, _expected in CASES],
)
def test_grammar_case(script: str, expected: tuple[tuple[str, ...], ...]) -> None:
    assert tokenize(script) == expected


@pytest.mark.parametrize("script", ['echo "never closed', "echo 'never closed"])
def test_unterminated_quote_refuses_rather_than_guessing(script: str) -> None:
    """Refusing is the point.

    A caller must not be able to read "nothing to see" out of "could not be
    read" -- the same fail-open shape these contracts exist to catch.
    """
    with pytest.raises(UnterminatedQuote):
        tokenize(script)


def test_every_shell_surface_in_the_repository_tokenizes() -> None:
    """The corpus, not a fixture.

    All three surfaces that carry shell: every workflow `run:` step, every
    tracked `.sh` file, and every Dockerfile `RUN` body. One module reads all
    of them, so a construct that appears in any one is caught here rather than
    when a guard is later pointed at that surface.

    Each surface found a distinct bug the others did not: `run:` steps found
    line-based lexing, the shell scripts found `2>&1`, and the Dockerfiles
    found command substitution.
    """
    root = Path(__file__).resolve().parents[2]
    sources = list(shellsurfaces.workflow_bodies(root / ".github" / "workflows").items())
    sources.extend(lint_harness.tracked_files(root, "*.sh")())
    sources.extend(
        shellsurfaces.dockerfile_bodies(
            root / "docker",
            root / "config" / "docker",
            lambda templates: shellsurfaces.rendered_templates(
                templates, root / "config" / "docker" / "image"
            ),
        ).items()
    )

    assert len(sources) > 230, "corpus looks truncated; this would pass vacuously"
    unreadable = []
    for where, script in sources:
        try:
            tokenize(script)
        except UnterminatedQuote:
            unreadable.append(where)
    assert not unreadable, f"cannot tokenize: {unreadable}"


def _workflow_fixture(root: Path, *, track_script: bool) -> Path:
    script = root / "scripts" / "phase.sh"
    script.parent.mkdir(parents=True)
    script.write_text("echo from-script\n", encoding="utf-8")
    workflow = root / ".github" / "workflows" / "fixture.yaml"
    workflow.parent.mkdir(parents=True)
    workflow.write_text(
        """\
jobs:
  lane:
    steps:
      - run: echo before
      - run: bash scripts/phase.sh
      - run: echo after
""",
        encoding="utf-8",
    )
    subprocess.run(["git", "init", "-q"], cwd=root, check=True)
    subprocess.run(["git", "add", str(workflow.relative_to(root))], cwd=root, check=True)
    if track_script:
        subprocess.run(["git", "add", str(script.relative_to(root))], cwd=root, check=True)
    return workflow


def test_workflow_reader_inlines_a_direct_script_at_its_execution_point(
    tmp_path: Path,
) -> None:
    workflow = _workflow_fixture(tmp_path, track_script=True)

    rendered = workflow_reachable_text(tmp_path, workflow, job="lane")

    assert rendered.index("echo before") < rendered.index("echo from-script")
    assert rendered.index("echo from-script") < rendered.index("echo after")


def test_workflow_reader_refuses_an_untracked_dispatch(tmp_path: Path) -> None:
    workflow = _workflow_fixture(tmp_path, track_script=False)

    with pytest.raises(AssertionError, match=r"dispatches untracked scripts/phase\.sh"):
        workflow_reachable_text(tmp_path, workflow, job="lane")
