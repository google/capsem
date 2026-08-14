"""Shell source to tokens, keeping enough structure to parse.

The repository asks questions of shell constantly -- which programs a script
runs, which arm of a dispatcher does what, whether a body stays inside the
boundary -- and every one of them was previously asked with a regular
expression. Each worked on the case it was written for and quietly failed on
the next: `cargo` in a filename, in a comment, on the left of an assignment,
or inside a quoted argument is not `cargo` in command position, and a pattern
that tells those apart has become a lexer with none of the testing.

So this is the lexer, and `shellast` is the parser above it. A word keeps its
raw text alongside its unquoted value, because callers need both: the value to
compare against a program name, the raw text to report back to a reader who has
to find it in the file.

Expansions -- `$(...)`, `${...}`, backticks -- are consumed as part of the word
they appear in rather than descended into. They are opaque by design: what a
command substitution evaluates to is a runtime question, and a tool that
guessed would be wrong in the direction that matters.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from enum import Enum, auto

#: A redirection, with its optional file descriptor and its optional `&fd`
#: target, as one token. Recognised here rather than in the parser because
#: only the lexer knows the parts were adjacent: `2>&1` and `2 > &1` are four
#: tokens either way once the words are split, and the first is a redirection
#: while the second passes `2` as an argument.
REDIRECT = re.compile(r"(?P<fd>\d+)?(?P<op><<-|<<|>>|>|<)(?P<target>&\d+|&-)?")

#: Operators, longest first so `;;` is never read as two `;` and `<<-` never as
#: `<<` followed by a stray dash.
OPERATORS = (
    "<<-",
    ";;",
    "&&",
    "||",
    ">>",
    "<<",
    ";",
    "|",
    "&",
    "(",
    ")",
    "<",
    ">",
)

_QUOTES = {"'", '"'}
_PAIRED = {"$(": ")", "${": "}", "`": "`"}


class Kind(Enum):
    """What a token is, structurally."""

    WORD = auto()
    OPERATOR = auto()
    NEWLINE = auto()


@dataclass(frozen=True)
class Token:
    kind: Kind
    text: str
    """The source text, quotes and all."""
    value: str
    """The text with one level of quoting removed, for comparison."""
    line: int

    @property
    def is_word(self) -> bool:
        return self.kind is Kind.WORD


class Lexer:
    """A single pass over one shell body.

    Deliberately not `shlex`. That module answers "split this into words",
    which loses the distinction between an operator and a word containing the
    same character, drops the raw text a diagnostic needs, and mangles command
    substitutions -- all three of which the parser above needs kept.
    """

    def __init__(self, source: str) -> None:
        self._source = source
        self._at = 0
        self._line = 1
        self._heredocs: list[str] = []

    def tokens(self) -> list[Token]:
        found: list[Token] = []
        while (token := self._next()) is not None:
            found.append(token)
        return found

    # -- scanning ----------------------------------------------------------

    def _peek(self, ahead: int = 0) -> str:
        index = self._at + ahead
        return self._source[index] if index < len(self._source) else ""

    def _next(self) -> Token | None:
        self._skip_blanks()
        if self._at >= len(self._source):
            return None

        char = self._peek()
        if char == "#":
            self._skip_to_end_of_line()
            return self._next()
        if char == "\n":
            self._at += 1
            line = self._line
            self._line += 1
            self._consume_heredoc_bodies()
            return Token(Kind.NEWLINE, "\n", "\n", line)
        # Redirections first, so an adjacent file descriptor stays attached.
        # `2>&1` is one redirection; `cargo build 2` passes an argument. The
        # difference is only adjacency, and this is the last place that knows.
        redirect = REDIRECT.match(self._source, self._at)
        if redirect is not None and redirect.start() == self._at and redirect.group(0):
            text = redirect.group(0)
            self._at += len(text)
            if redirect.group("op") in {"<<", "<<-"}:
                self._arm_heredoc()
            return Token(Kind.OPERATOR, text, text, self._line)
        for operator in OPERATORS:
            if self._source.startswith(operator, self._at):
                self._at += len(operator)
                return Token(Kind.OPERATOR, operator, operator, self._line)
        return self._word()

    def _skip_blanks(self) -> None:
        while self._at < len(self._source):
            char = self._peek()
            if char in " \t\r":
                self._at += 1
            elif char == "\\" and self._peek(1) == "\n":
                self._at += 2
                self._line += 1
            else:
                return

    def _skip_to_end_of_line(self) -> None:
        while self._at < len(self._source) and self._peek() != "\n":
            self._at += 1

    def _word(self) -> Token:
        """One word, with quoting and expansions consumed as part of it."""
        start = self._at
        line = self._line
        value: list[str] = []
        while self._at < len(self._source):
            char = self._peek()
            if char in " \t\r\n":
                break
            if any(self._source.startswith(operator, self._at) for operator in OPERATORS):
                break
            if char == "\\":
                value.append(self._peek(1))
                self._at += 2
                continue
            if char in _QUOTES:
                value.append(self._quoted(char))
                continue
            pair = next(
                (opening for opening in _PAIRED if self._source.startswith(opening, self._at)),
                None,
            )
            if pair is not None:
                value.append(self._expansion(pair))
                continue
            value.append(char)
            self._at += 1
        return Token(Kind.WORD, self._source[start : self._at], "".join(value), line)

    def _quoted(self, quote: str) -> str:
        self._at += 1
        collected: list[str] = []
        while self._at < len(self._source) and self._peek() != quote:
            if quote == '"' and self._peek() == "\\":
                collected.append(self._peek(1))
                self._at += 2
                continue
            if self._peek() == "\n":
                self._line += 1
            collected.append(self._peek())
            self._at += 1
        self._at += 1  # the closing quote, or end of input on an unbalanced one
        return "".join(collected)

    def _expansion(self, opening: str) -> str:
        """A `$(...)`, `${...}` or backtick run, consumed whole and opaque.

        Nesting is counted for the bracketed forms, so `$(dirname $(pwd))` is
        one word rather than a word and a stray parenthesis that would look to
        the parser like the end of a subshell.
        """
        closing = _PAIRED[opening]
        start = self._at
        self._at += len(opening)
        depth = 1
        while self._at < len(self._source) and depth:
            if opening != "`" and self._source.startswith(opening, self._at):
                depth += 1
                self._at += len(opening)
                continue
            if self._peek() == closing:
                depth -= 1
                self._at += 1
                continue
            if self._peek() == "\n":
                self._line += 1
            self._at += 1
        return self._source[start : self._at]

    # -- heredocs ----------------------------------------------------------

    def _arm_heredoc(self) -> None:
        """Remember the delimiter that follows `<<`, to skip its body later."""
        save = self._at
        self._skip_blanks()
        token = self._word()
        if token.value:
            self._heredocs.append(token.value)
        else:
            self._at = save

    def _consume_heredoc_bodies(self) -> None:
        """Skip each pending heredoc body.

        The body is data, not shell. Lexing it produced commands nothing runs
        -- and, when the data happened to contain a quote, an unbalanced string
        that swallowed the rest of the file.
        """
        while self._heredocs:
            delimiter = self._heredocs.pop(0)
            while self._at < len(self._source):
                end = self._source.find("\n", self._at)
                line = self._source[self._at : end if end != -1 else len(self._source)]
                self._at = len(self._source) if end == -1 else end + 1
                self._line += 1
                if line.strip() == delimiter:
                    break


def tokenize(source: str) -> list[Token]:
    return Lexer(source).tokens()
