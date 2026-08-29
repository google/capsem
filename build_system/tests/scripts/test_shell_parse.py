"""The shell lexer and parser, exercised on the constructs that break them.

Written against the corners rather than the happy path, because the happy path
is what every regular expression this replaces already handled. Each group
below is a way one of those patterns was wrong: a word that contains an
operator, a comment that is not a comment, an arm that runs into the next one,
a conjunction that discards a verdict.
"""

from __future__ import annotations

import warnings

import pytest
from capsem_builder.gate.shelllex import Kind, heredocs, tokenize
from capsem_builder.gate.shellnodes import (
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
from capsem_builder.gate.shellparse import parse
from capsem_builder.gate.shellsniff import ForeignSourceWarning, sniff


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


def test_heredoc_metadata_preserves_quotedness_body_and_line() -> None:
    source = "cat <<EOF\n`runs`\nEOF\ncat <<'SAFE'\n$(does-not-run)\nSAFE\n"

    unquoted, quoted = heredocs(source)

    assert (unquoted.delimiter, unquoted.quoted, unquoted.body) == (
        "EOF",
        False,
        ((2, "`runs`"),),
    )
    assert (quoted.delimiter, quoted.quoted, quoted.body) == (
        "SAFE",
        True,
        ((5, "$(does-not-run)"),),
    )


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
    command = commands(parse("pnpm --dir web/app run build"))[0]
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
    parsed = parse("(cd web/app && npx vitest run)\ncargo build")
    assert any(isinstance(node, Compound) for node in parsed)
    assert programs("(cd web/app && npx vitest run)\ncargo build") == [
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
        pnpm --dir web/docs run build
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


# -- the corners that bite ---------------------------------------------------


@pytest.mark.parametrize(
    ("source", "expected"),
    [
        ("echo $((1 + 2))", ["echo", "$((1 + 2))"]),
        ("echo ${x:-$(pwd)}", ["echo", "${x:-$(pwd)}"]),
        ('echo "$(cargo metadata)"', ["echo", "$(cargo metadata)"]),
        ("echo 'a)b'", ["echo", "a)b"]),
        ('echo "a;b"', ["echo", "a;b"]),
        ("echo \"it's\"", ["echo", "it's"]),
        ("echo a=b", ["echo", "a=b"]),
    ],
)
def test_expansions_and_quotes_stay_one_word(source: str, expected: list[str]) -> None:
    """A delimiter that closes early leaves a stray one behind.

    `echo 'a)b'` is the case that matters for `case`: read unquoted, that `)`
    ends the arm and everything after it belongs to the next one.
    """
    assert [token.value for token in tokenize(source) if token.is_word] == expected


def test_a_command_substitution_is_not_descended_into() -> None:
    """Opaque by design: what it evaluates to is a runtime question.

    It is reported as part of its word, so a caller can see `$(cargo build)`
    and decide. It is not silently counted as an invocation, which would make
    the answer depend on a guess.
    """
    assert programs("X=$(cargo build) echo done") == ["echo"]
    assert argvs("echo $(cargo build)") == [("echo", "$(cargo build)")]


def test_a_case_inside_a_case_keeps_its_arms_apart() -> None:
    source = """\
case "$1" in
  outer)
    case "$2" in
      inner) cargo build ;;
      other) pnpm run build ;;
    esac
    ;;
  sibling)
    echo sibling
    ;;
esac
"""
    tree = parse(source)
    assert [c.program for c in commands(arm_named(tree, "inner") or [])] == ["cargo"]
    assert [c.program for c in commands(arm_named(tree, "other") or [])] == ["pnpm"]
    assert [c.program for c in commands(arm_named(tree, "sibling") or [])] == ["echo"]


def test_a_word_that_looks_like_a_keyword_is_not_one() -> None:
    """`esac`, `fi` and `done` as arguments must not close anything."""
    assert argvs("echo esac fi done") == [("echo", "esac", "fi", "done")]
    assert programs("grep -q 'case x in' file") == ["grep"]


def test_a_conjunction_chain_keeps_every_link() -> None:
    assert programs("a && b && c") == ["a", "b", "c"]
    assert programs("a || b || c") == ["a", "b", "c"]
    assert programs("a && b || c && d") == ["a", "b", "c", "d"]


def test_a_conjunction_split_across_lines_is_one_chain() -> None:
    """`&&` at end of line continues without a backslash."""
    source = "cargo build &&\n  cargo test"
    assert programs(source) == ["cargo", "cargo"]
    assert [node for node in walk(parse(source)) if isinstance(node, AndOr)]


def test_suppression_is_not_confused_by_nesting() -> None:
    """`|| true` inside a function or an arm still discards a verdict, and a
    `true` that is not on the right of `||` discards nothing."""
    assert [c.program for c in suppressed(parse("f() { risky || true; }"))] == ["risky"]
    assert [c.program for c in suppressed(parse("case $1 in a) risky || true ;; esac"))] == [
        "risky"
    ]
    assert suppressed(parse("true && risky")) == []
    assert suppressed(parse("risky; true")) == []


def test_a_pipeline_on_the_left_of_a_suppression_is_reported_whole() -> None:
    assert [c.program for c in suppressed(parse("check | grep -q ok || true"))] == [
        "check",
        "grep",
    ]


def test_redirection_targets_are_not_mistaken_for_commands() -> None:
    assert programs("cargo build > cargo") == ["cargo"]
    assert programs("echo x 2> /dev/null") == ["echo"]
    assert argvs("echo x >&2") == [("echo", "x")]


def test_a_semicolon_inside_a_string_does_not_split() -> None:
    assert argvs('echo "a; cargo build"') == [("echo", "a; cargo build")]


def test_a_backslash_in_single_quotes_is_literal() -> None:
    assert [t.value for t in tokenize(r"echo 'a\b'") if t.is_word] == ["echo", r"a\b"]


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

    root = Path(__file__).resolve().parents[3]
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


# -- being handed the wrong thing --------------------------------------------


def test_a_container_of_shell_is_not_shell() -> None:
    """The mistake this catches was made twice in the hour it was written.

    A raw `.j2` template and a whole Dockerfile both lex without error and
    produce confident nonsense -- which is worse than an exception, because the
    guard reading the result reports a clean tree.
    """
    assert sniff("FROM ubuntu:24.04\nRUN apt-get update\n") == "dockerfile"
    assert sniff("{% if arch %}\nmake\n{% endif %}\n") == "jinja"
    assert sniff("jobs:\n  build:\n    runs-on: ubuntu\n") == "yaml"


def test_the_sniffer_does_not_guess() -> None:
    """A false positive here is worse than no sniffer.

    `docker inspect --format '{{range .Mounts}}'` is a Go template inside
    perfectly good shell, and this repository runs exactly that. `{{` can
    therefore never be the Jinja signal; `{%` can.
    """
    assert sniff("docker inspect --format '{{range .Mounts}}{{println .Source}}{{end}}' $id") is None
    assert sniff("set -eux\napt-get update && apt-get install -y curl\n") is None
    assert sniff("echo 'FROM the top'") is None, "FROM must be at line start to count"
    assert sniff("python3 - <<'PY'\nimport json\nPY\n") is None, (
        "eight tracked scripts embed a Python heredoc; that is shell"
    )


def test_the_warning_names_the_language_and_the_origin() -> None:
    with pytest.warns(ForeignSourceWarning, match="jinja"):
        parse("{% if arch %}\nmake\n{% endif %}\n", origin="Dockerfile.kernel.j2")


def test_a_caller_that_knows_better_can_say_so() -> None:
    with warnings.catch_warnings():
        warnings.simplefilter("error")
        assert parse("FROM ubuntu\nRUN x\n", foreign="allow") is not None
    with pytest.raises(ValueError, match="dockerfile"):
        parse("FROM ubuntu\nRUN x\n", foreign="refuse")
