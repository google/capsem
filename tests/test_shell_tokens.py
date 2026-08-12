"""The shell tokenizer's grammar, case by case.

Each case is a line of the grammar in `helpers/shelltokens.py`. The two marked
REGRESSION are the bugs the previous line-based implementation had, kept as
cases so a future rewrite has to answer them.
"""

from __future__ import annotations

import pytest
from helpers.shelltokens import UnterminatedQuote, tokenize

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


def test_every_tracked_script_and_workflow_step_tokenizes() -> None:
    """The corpus, not a fixture: 46 shell scripts and every `run:` step.

    The previous implementation raised on four of them, all in release.yaml.
    """
    import pathlib
    import subprocess

    import yaml

    root = pathlib.Path(__file__).resolve().parent.parent
    sources: list[tuple[str, str]] = []
    for path in sorted((root / ".github/workflows").glob("*.yaml")):
        document = yaml.safe_load(path.read_text()) or {}
        for job_name, job in (document.get("jobs") or {}).items():
            for step in job.get("steps") or []:
                if isinstance(step, dict) and step.get("run"):
                    sources.append((f"{path.name}:{job_name}", str(step["run"])))
    listed = subprocess.run(
        ["git", "ls-files", "--", "*.sh"],
        cwd=root,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.split()
    sources.extend((name, (root / name).read_text()) for name in listed)

    assert len(sources) > 200, "corpus looks truncated; this would pass vacuously"
    unreadable = []
    for where, script in sources:
        try:
            tokenize(script)
        except UnterminatedQuote:
            unreadable.append(where)
    assert not unreadable, f"cannot tokenize: {unreadable}"
