"""Read the shell a Python script emits, and read it as shell.

Guards over this repository's scripts kept being written with regexes and
`str.index`, and kept being wrong in the same two ways.

They were wrong about *what* they were reading. A Python file that emits bash
through f-strings contains `service_logs() {{`; the shell that runs contains
`service_logs() {`. Every guard doing `source.index("...{{")` was matching the
Python escaping rather than the program, so it silently stopped matching the
day a fragment moved into a plain string, and it could never see a fragment
built by concatenation.

They were wrong about *how*. `tail|cat|head` as a regex missed `grep`;
`NAME=...` missed a function returning the same path. Each hole was found by
shipping the bug it was meant to catch. A tokeniser does not have holes of
that shape: a command is a verb and its arguments whatever spacing, quoting
and line continuation were used to write it.

So: `ast` decides what the shell is, and `shlex` decides what the shell says.
"""

from __future__ import annotations

import ast
import shlex
from pathlib import Path

#: Tokens `shlex` yields for operators, which end one command and start the
#: next. `punctuation_chars` makes these tokens rather than word characters.
_SEPARATORS = {";", "|", "||", "&", "&&", "\n", "(", ")", "{", "}"}


def emitted_shell(source: str) -> str:
    """Every string literal in a Python module, as its value rather than its source.

    `ast` resolves the escaping for us: an f-string written `{{` has the value
    `{`. That is the whole reason to parse rather than read -- the text a guard
    must judge is the text the shell receives, and those differ exactly where
    shell syntax is densest.

    F-string placeholders are dropped, so `"$X/{name}.log"` reads as
    `"$X/.log"`. A guard must therefore not depend on an interpolated value,
    which is correct: it cannot know one.
    """
    parts: list[str] = []
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, ast.JoinedStr):
            parts.append(
                "".join(
                    value.value
                    for value in node.values
                    if isinstance(value, ast.Constant) and isinstance(value.value, str)
                )
            )
        elif isinstance(node, ast.Constant) and isinstance(node.value, str):
            parts.append(node.value)
    return "\n".join(parts)


def logical_lines(text: str) -> list[str]:
    """Shell lines, with backslash continuations joined.

    A command spread over three lines is one command. Every regex guard here
    had to pretend otherwise, which is why they all carried `[^\n|]*?` and all
    missed anything wrapped for line length.
    """
    joined, buffer = [], ""
    for line in text.splitlines():
        buffer += line
        if buffer.endswith("\\"):
            buffer = buffer[:-1] + " "
            continue
        joined.append(buffer)
        buffer = ""
    if buffer:
        joined.append(buffer)
    return joined


def commands(text: str) -> list[list[str]]:
    """Tokenise shell into commands, each a verb followed by its words.

    Quoting, spacing, comments and line continuation stop mattering, which is
    the point: `grep -Fq "$needle" "$log"` and `grep  -Fq  $needle  $log`
    spread over two lines are the same command, and a regex has to be told so
    every time.
    """
    found: list[list[str]] = []
    for line in logical_lines(text):
        lexer = shlex.shlex(line, posix=True, punctuation_chars=True)
        lexer.whitespace_split = True
        lexer.commenters = "#"
        current: list[str] = []
        try:
            for token in lexer:
                if token in _SEPARATORS:
                    if current:
                        found.append(current)
                    current = []
                else:
                    current.append(token)
        except ValueError:
            # An unbalanced quote in a fragment reconstructed from f-string
            # literals. What tokenised before it is still worth judging.
            pass
        if current:
            found.append(current)
    return found


def function_bodies(text: str) -> dict[str, str]:
    """`name() { ... }` in shell, by name.

    Brace-counted rather than matched with a regex, so a body containing a
    brace -- `${VAR}`, a compound command -- does not truncate it. The pattern
    a regex guard used stopped at the first `}` and would have read
    `f() { echo "${HOME}/x.log"; }` as ending after `${HOME`.
    """
    bodies: dict[str, str] = {}
    for index, line in enumerate(lines := text.splitlines()):
        stripped = line.strip()
        if not stripped.endswith("{") or "()" not in stripped:
            continue
        name = stripped.split("(")[0].strip()
        if not name.replace("_", "").replace("-", "").isalnum():
            continue
        depth, collected = 0, []
        for following in lines[index:]:
            depth += following.count("{") - following.count("}")
            collected.append(following)
            if depth <= 0:
                break
        bodies[name] = "\n".join(collected)
    return bodies


def shell_of(path: Path) -> str:
    """The shell a file contains, whether it is a shell script or emits one."""
    text = path.read_text(encoding="utf-8", errors="replace")
    return emitted_shell(text) if path.suffix == ".py" else text
