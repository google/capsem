"""The shell lexer and parser, exercised on the constructs that break them.

Written against the corners rather than the happy path, because the happy path
is what every regular expression this replaces already handled. Each group
below is a way one of those patterns was wrong: a word that contains an
operator, a comment that is not a comment, an arm that runs into the next one,
a conjunction that discards a verdict.
"""

from __future__ import annotations

import pytest

from capsem.gate.shelllex import Kind, tokenize
from capsem.gate.shellnodes import (
    AndOr,
    Command,
    Compound,
    Function,
    Pipeline,
    arm_named,
    commands,
    suppressed,
    walk,
)
from capsem.gate.shellparse import parse


def programs(source: str) -> list[str]:
    return [command.program for command in commands(parse(source))]


def argvs(source: str) -> list[tuple[str, ...]]:
    return [command.argv for command in commands(parse(source))]


# -- lexing -----------------------------------------------------------------


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        ("echo hello", ["echo", "hello"]),
        ("echo 'a b'", ["echo", "a b"]),
        ('echo "a b"', ["echo", "a b"]),
        (r"echo a\ b", ["echo", "a b"]),
        ('echo "he said \\"hi\\""', ["echo", 'he said "hi"']),
        ("echo 'it'\"'\"'s'", ["echo", "it's"]),
        ("echo a#b", ["echo", "a#b"]),
        ("echo $(date +%s)", ["echo", "$(date +%s)"]),
        ("echo $(dirname $(pwd))", ["echo", "$(dirname $(pwd))"]),
        ("echo ${HOME}/x", ["echo", "${HOME}/x"]),
        ("echo `uname`", ["echo", "`uname`"]),
    ],
)
def test_words_survive_quoting_and_expansion(source: str, expected: list[str]) -> None:
    """A word is one token however it is spelled.

    `$(dirname $(pwd))` is the case that matters most: an unbalanced read of it
    leaves a `)` that the parser takes for the end of a subshell, and every
    command after it lands in the wrong scope.
    """
    assert [token.value for token in tokenize(source) if token.is_word] == expected


def test_a_comment_is_only_a_comment_in_word_position() -> None:
    assert programs("# cargo build") == []
    assert programs("echo '# cargo build'") == ["echo"]
    assert argvs("echo a#b") == [("echo", "a#b")]
    assert programs("cargo build  # then this") == ["cargo"]


def test_operators_are_not_confused_with_their_prefixes() -> None:
    """`;;` is not two `;`, `&&` is not two `&`, `>>` is not two `>`."""
    kinds = {
        token.text
        for token in tokenize("a ;; b && c || d | e & f >> g")
        if token.kind is Kind.OPERATOR
    }
    assert {";;", "&&", "||", "|", "&", ">>"} <= kinds


def test_a_continuation_joins_one_command() -> None:
    assert argvs("cargo run \\\n  -p admin \\\n  -- build") == [
        ("cargo", "run", "-p", "admin", "--", "build")
    ]


def test_a_heredoc_body_is_data_not_shell() -> None:
    """Its contents must not be read as commands, and must not unbalance.

    The body regularly contains apostrophes. Lexing it as shell consumed the
    rest of the file into one unterminated string.
    """
    source = "cat <<'EOF'\ncargo build -p nope\nit's fine\nEOF\ncargo run -p yes\n"
    assert programs(source) == ["cat", "cargo"]
    assert argvs(source)[1] == ("cargo", "run", "-p", "yes")


def test_an_indented_heredoc_terminator_is_honoured() -> None:
    source = "cat <<-EOF\n\tbody\n\tEOF\necho after\n"
    assert programs(source) == ["cat", "echo"]


def test_an_unbalanced_quote_does_not_hang_or_raise() -> None:
    assert isinstance(parse("echo 'unterminated\ncargo build\n"), list)


# -- simple commands --------------------------------------------------------


def test_an_assignment_is_not_a_command() -> None:
    """The case that made a Python file look like it ran cargo."""
    parsed = commands(parse("cargo=1\nCARGO_TARGET=/tmp cargo build"))
    assert [command.program for command in parsed] == ["cargo"]
    assert parsed[0].assignments == ("CARGO_TARGET=/tmp",)
    assert parsed[0].argv == ("cargo", "build")


def test_wrappers_report_the_command_they_run() -> None:
    assert programs("env FOO=1 cargo build") == ["cargo"]
    assert programs("sudo docker ps") == ["docker"]
    assert programs("caffeinate env X=1 cargo test") == ["cargo"]


def test_a_program_named_in_an_argument_is_not_a_program() -> None:
    assert programs("pytest tests/test_cargo_build.py") == ["pytest"]
    assert programs('echo "cargo build"') == ["echo"]


def test_redirections_do_not_become_arguments() -> None:
    assert argvs("cargo build > out.log 2>&1") == [("cargo", "build")]
    assert argvs("grep -q x < in.txt") == [("grep", "-q", "x")]


def test_subcommand_skips_options_and_their_values() -> None:
    command = commands(parse("pnpm --dir frontend run build"))[0]
    assert command.subcommand(after="--dir") == "run"
    assert commands(parse("cargo run -p x"))[0].subcommand() == "run"


# -- composition ------------------------------------------------------------


def test_a_pipeline_keeps_every_stage() -> None:
    parsed = parse("cargo metadata | jq .packages | head -1")
    assert any(isinstance(node, Pipeline) for node in parsed)
    assert programs("cargo metadata | jq .packages | head -1") == ["cargo", "jq", "head"]


def test_a_conjunction_is_a_node_not_a_list() -> None:
    parsed = parse("make && ./run || echo failed")
    assert any(isinstance(node, AndOr) for node in walk(parsed))
    assert programs("make && ./run || echo failed") == ["make", "./run", "echo"]


def test_a_discarded_verdict_is_visible() -> None:
    """`|| true` is the shape that passed a contract while the check failed."""
    assert [c.program for c in suppressed(parse('test "$X" = success || true'))] == ["test"]
    assert [c.program for c in suppressed(parse("check-branch-protection || :"))] == [
        "check-branch-protection"
    ]
    assert suppressed(parse("check || fallback")) == [], "a real fallback is not suppression"
    assert suppressed(parse("check && true")) == [], "only || discards the verdict"


def test_a_subshell_is_scoped() -> None:
    parsed = parse("(cd frontend && npx vitest run)\ncargo build")
    assert any(isinstance(node, Compound) for node in parsed)
    assert programs("(cd frontend && npx vitest run)\ncargo build") == [
        "cd",
        "npx",
        "cargo",
    ]


# -- case --------------------------------------------------------------------

DISPATCHER = """\
case "$1" in
    build)
        cargo build
        ;;
    check|lint)
        cargo clippy
        ;;
    (docs)
        pnpm --dir docs run build
        ;;
    *)
        echo usage >&2
        exit 1
        ;;
esac
"""


def test_each_arm_holds_only_its_own_commands() -> None:
    """The bug that made every arm report every other arm's work.

    `;;` is separator-shaped, so skipping it as one ran the whole dispatcher
    together and the first arm appeared to run all of it.
    """
    tree = parse(DISPATCHER)
    assert [c.program for c in commands(arm_named(tree, "build") or [])] == ["cargo"]
    assert [c.program for c in commands(arm_named(tree, "docs") or [])] == ["pnpm"]
    assert [c.program for c in commands(arm_named(tree, "check") or [])] == ["cargo"]


def test_an_arm_can_carry_several_patterns() -> None:
    tree = parse(DISPATCHER)
    assert arm_named(tree, "check") == arm_named(tree, "lint")


def test_a_parenthesised_pattern_is_the_same_arm() -> None:
    assert arm_named(parse(DISPATCHER), "docs") is not None


def test_a_missing_arm_is_none_not_empty() -> None:
    """The distinction a caller must not lose: no such sub-command, versus a
    sub-command that does nothing."""
    assert arm_named(parse(DISPATCHER), "nope") is None
    assert arm_named(parse("case $1 in empty)\n ;;\nesac"), "empty") == []


def test_the_default_arm_is_reachable_by_its_pattern() -> None:
    assert [c.program for c in commands(arm_named(parse(DISPATCHER), "*") or [])] == [
        "echo",
        "exit",
    ]


# -- compounds ---------------------------------------------------------------


def test_a_function_body_is_attributed_to_the_function() -> None:
    tree = parse("build() {\n  cargo build\n}\nbuild\n")
    functions = [node for node in walk(tree) if isinstance(node, Function)]
    assert [node.name for node in functions] == ["build"]
    assert [c.program for c in commands(functions[0].body)] == ["cargo"]


def test_the_keyword_form_of_a_function_parses() -> None:
    tree = parse("function build {\n  cargo build\n}\n")
    assert [node.name for node in walk(tree) if isinstance(node, Function)] == ["build"]


@pytest.mark.parametrize(
    "source",
    [
        "if [ -f x ]; then cargo build; fi",
        "for f in a b; do cargo build; done",
        "while read -r line; do cargo build; done",
        "until false; do cargo build; done",
    ],
)
def test_a_compound_body_is_still_searchable(source: str) -> None:
    assert "cargo" in programs(source)


def test_keywords_are_not_reported_as_programs() -> None:
    found = programs("if [ -f x ]; then cargo build; else echo no; fi")
    assert "then" not in found and "fi" not in found and "else" not in found
    assert found == ["[", "cargo", "echo"]


# -- robustness --------------------------------------------------------------


@pytest.mark.parametrize(
    "source",
    ["", "\n\n", "esac", "fi", ";;", "case", "case $x in", "}", ")", "&& echo x"],
)
def test_malformed_input_yields_a_tree_rather_than_an_exception(source: str) -> None:
    """A guard that raises on an unfamiliar construct gets deleted."""
    assert isinstance(parse(source), list)


def test_every_tracked_script_parses() -> None:
    """The real corpus, which is the only proof that matters."""
    import subprocess
    from pathlib import Path

    root = Path(__file__).resolve().parents[1]
    listed = subprocess.run(
        ["git", "ls-files", "*.sh"], cwd=root, capture_output=True, text=True, check=True
    ).stdout.split()
    assert len(listed) > 10, "scanned too few scripts to trust this"
    for name in listed:
        tree = parse((root / name).read_text(encoding="utf-8"))
        assert isinstance(tree, list), name
        found = commands(tree)
        assert found, f"{name} parsed to no commands at all"
        assert all(isinstance(item, Command) for item in found)
