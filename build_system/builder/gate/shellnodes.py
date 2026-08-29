"""The shape of a shell script: nodes, and the questions worth asking of one.

The tree is what makes those questions exact. "Which programs does the
`release-site` arm run" is a walk of one `Arm`, not a pattern over a file with
an approximate idea of where the arm ends. "Does this step build Rust" is
`Command.program == "cargo"` in command position, which no amount of pattern
refinement gets right, because the distinction is grammatical rather than
textual.

`shellparse` builds these; this module only describes and interrogates them,
which is why a consumer that reads a tree never imports the parser.
"""

from __future__ import annotations

from collections.abc import Iterator
from dataclasses import dataclass, field

#: Words that introduce a compound and are never a program being run.
KEYWORDS = frozenset(
    {
        "if",
        "then",
        "elif",
        "else",
        "fi",
        "for",
        "while",
        "until",
        "do",
        "done",
        "case",
        "esac",
        "in",
        "function",
        "select",
        "time",
        "!",
        "{",
        "}",
    }
)

#: Wrappers whose argument is the command that actually runs.
WRAPPERS = frozenset({"env", "sudo", "exec", "caffeinate", "nice", "command", "xargs"})

#: What ends a command. `;;` is deliberately absent: it terminates a `case`
#: arm, and treating it as an ordinary separator made every arm of
#: `check-web-surface.sh` parse as one, which reported the `frontend-verify`
#: arm as running docs, site and the release channel.
SEPARATORS = {";", "&", "\n"}


@dataclass(frozen=True)
class Command:
    """One simple command: its assignments, its argv, and where it was."""

    argv: tuple[str, ...]
    assignments: tuple[str, ...]
    line: int

    @property
    def program(self) -> str:
        """The program this runs, stepping over wrappers and their options.

        `env FOO=1 cargo build` runs cargo, and a caller asking "what does this
        run" means cargo. Reporting the wrapper is technically true and useless.
        """
        for index, word in enumerate(self.argv):
            if word in WRAPPERS or "=" in word.split("/")[0]:
                continue
            return self.argv[index]
        return ""

    def subcommand(self, *, after: str | None = None) -> str:
        """The first non-option word following the program.

        `after` names a flag that takes a value, so `pnpm --dir web/app run`
        can be asked for `run` rather than `frontend`.
        """
        rest = (
            list(self.argv[self.argv.index(self.program) + 1 :])
            if self.program in self.argv
            else []
        )
        skip = False
        for word in rest:
            if skip:
                skip = False
                continue
            if word.startswith("-"):
                skip = word == after
                continue
            return word
        return ""


@dataclass
class Arm:
    """One `case` branch: the patterns it matches and what it then runs."""

    patterns: tuple[str, ...]
    body: list[Node] = field(default_factory=list)

    def matches(self, name: str) -> bool:
        return name in self.patterns


@dataclass
class Case:
    subject: str
    arms: list[Arm] = field(default_factory=list)


@dataclass
class Compound:
    """A subshell, group, loop or conditional: a body with a kind."""

    keyword: str
    body: list[Node] = field(default_factory=list)


@dataclass
class Function:
    name: str
    body: list[Node] = field(default_factory=list)


@dataclass
class Pipeline:
    """`a | b | c`. Only the last command's status survives."""

    parts: list[Node] = field(default_factory=list)


@dataclass
class AndOr:
    """`left && right` or `left || right`.

    Modelled rather than flattened, because the operator is the whole meaning.
    `test "$X" = success || true` runs the test and then throws its verdict
    away; flattened to a list of commands it looks identical to a script that
    checks the thing. That exact shape satisfied a release contract while
    branch protection was switched off.
    """

    operator: str
    left: Node
    right: Node


Node = Command | Case | Compound | Function | Pipeline | AndOr


def walk(nodes: list[Node]) -> Iterator[Node]:
    """Every node in the tree, parents before children."""
    for node in nodes:
        yield node
        if isinstance(node, Compound | Function):
            yield from walk(node.body)
        elif isinstance(node, Case):
            for arm in node.arms:
                yield from walk(arm.body)
        elif isinstance(node, Pipeline):
            yield from walk(node.parts)
        elif isinstance(node, AndOr):
            yield from walk([node.left, node.right])


#: Commands that succeed unconditionally. On the right of `||` they discard
#: whatever the left side concluded.
SWALLOWS = frozenset({"true", ":"})


def suppressed(nodes: list[Node]) -> list[Command]:
    """Commands whose failure is discarded by `|| true` or `|| :`.

    The question a reviewer means by "does this actually check anything".
    """
    found: list[Command] = []
    for node in walk(nodes):
        if not isinstance(node, AndOr) or node.operator != "||":
            continue
        tail = node.right
        if isinstance(tail, Command) and tail.program in SWALLOWS:
            found.extend(item for item in walk([node.left]) if isinstance(item, Command))
    return found


def commands(nodes: list[Node]) -> list[Command]:
    return [node for node in walk(nodes) if isinstance(node, Command)]


def arm_named(nodes: list[Node], name: str) -> list[Node] | None:
    """The body of the `case` arm `name` selects, anywhere in the tree.

    `None` when no arm matches, which a caller must not confuse with an empty
    body: the first means the sub-command does not exist, the second means it
    exists and does nothing.
    """
    for node in walk(nodes):
        if isinstance(node, Case):
            for arm in node.arms:
                if arm.matches(name):
                    return arm.body
    return None
