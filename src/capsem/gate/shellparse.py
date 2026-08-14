"""Tokens to a tree: a recursive-descent parser for the shell we actually have.

Simple commands, pipelines, `&&`/`||` lists, subshells and groups, `if`, `for`,
`while`/`until`, `case`, and function definitions. Not a POSIX shell -- it does
not have to run anything, only to say what is where.

Anything unparseable degrades to the nodes found so far rather than raising. A
guard that crashes on an unfamiliar construct gets deleted; one that reports
what it could read stays useful, and honest about the rest.
"""

from __future__ import annotations

from .shelllex import REDIRECT, Kind, Token, tokenize
from .shellnodes import (
    KEYWORDS,
    SEPARATORS,
    AndOr,
    Arm,
    Case,
    Command,
    Compound,
    Function,
    Node,
    Pipeline,
)


class Parser:
    def __init__(self, tokens: list[Token]) -> None:
        self._tokens = tokens
        self._at = 0

    def parse(self, until: frozenset[str] = frozenset()) -> list[Node]:
        nodes: list[Node] = []
        while (token := self._peek()) is not None:
            # The terminator is tested first. `;;` is both an arm terminator
            # and separator-shaped, and skipping it as a separator ran every
            # `case` arm together into the first one.
            if token.value in until:
                return nodes
            if token.kind is Kind.NEWLINE or token.value in SEPARATORS:
                self._at += 1
                continue
            before = self._at
            node = self._andor(until)
            if node is not None:
                nodes.append(node)
            if self._at == before:
                self._at += 1  # unparseable: step over rather than spin
        return nodes

    def _andor(self, until: frozenset[str]) -> Node | None:
        left = self._pipeline(until)
        while (token := self._peek()) is not None and token.value in {"&&", "||"}:
            operator = token.value
            self._at += 1
            self._skip_newlines()
            right = self._pipeline(until)
            if left is None:
                left = right
            elif right is not None:
                left = AndOr(operator, left, right)
        return left

    def _pipeline(self, until: frozenset[str]) -> Node | None:
        first = self._node()
        if first is None:
            return None
        parts = [first]
        while (token := self._peek()) is not None and token.value == "|":
            self._at += 1
            self._skip_newlines()
            if (nxt := self._node()) is not None:
                parts.append(nxt)
        return parts[0] if len(parts) == 1 else Pipeline(parts)

    def _skip_newlines(self) -> None:
        while (token := self._peek()) is not None and token.kind is Kind.NEWLINE:
            self._at += 1

    # -- pieces ------------------------------------------------------------

    def _peek(self, ahead: int = 0) -> Token | None:
        index = self._at + ahead
        return self._tokens[index] if index < len(self._tokens) else None

    def _node(self) -> Node | None:
        token = self._peek()
        if token is None:
            return None
        word = token.value
        if word == "case":
            return self._case()
        if word in {"if", "while", "until", "for", "select"}:
            return self._compound(word)
        if word in {"(", "{"}:
            return self._group(word)
        if self._is_function():
            return self._function()
        if word in KEYWORDS:
            self._at += 1
            return None
        return self._simple()

    def _is_function(self) -> bool:
        token, after, then = self._peek(), self._peek(1), self._peek(2)
        if token is None or not token.is_word:
            return False
        if token.value == "function":
            return True
        return bool(after and after.value == "(" and then and then.value == ")")

    def _function(self) -> Function:
        token = self._peek()
        # `function build { ... }` names the function in the *next* word; the
        # keyword form was reporting every such function as named "function".
        if token is not None and token.value == "function":
            self._at += 1
            token = self._peek()
        name = token.value if token is not None else ""
        self._at += 1
        while (nxt := self._peek()) is not None and nxt.value in {"(", ")", "\n"}:
            self._at += 1
        return Function(name, self._body_of({"}"}))

    def _group(self, opening: str) -> Compound:
        self._at += 1
        closing = ")" if opening == "(" else "}"
        body = self.parse(frozenset({closing}))
        self._at += 1
        return Compound(opening, body)

    def _compound(self, keyword: str) -> Compound:
        self._at += 1
        terminator = {"if": "fi", "for": "done", "while": "done", "until": "done", "select": "done"}
        return Compound(keyword, self.parse(frozenset({terminator[keyword]})) + self._close())

    def _close(self) -> list[Node]:
        self._at += 1
        return []

    def _body_of(self, until: set[str]) -> list[Node]:
        while (token := self._peek()) is not None and token.value not in until | {"{", "("}:
            self._at += 1
        if (token := self._peek()) is not None and token.value in {"{", "("}:
            return self._group(token.value).body
        return []

    def _case(self) -> Case:
        self._at += 1
        subject = ""
        if (token := self._peek()) is not None:
            subject = token.value
            self._at += 1
        while (token := self._peek()) is not None and token.value != "in":
            self._at += 1
        self._at += 1
        node = Case(subject)
        while (token := self._peek()) is not None and token.value != "esac":
            if token.kind is Kind.NEWLINE or token.value in {";;", ";"}:
                self._at += 1
                continue
            node.arms.append(self._arm())
        self._at += 1
        return node

    def _arm(self) -> Arm:
        patterns: list[str] = []
        if (token := self._peek()) is not None and token.value == "(":
            self._at += 1
        while (token := self._peek()) is not None and token.value != ")":
            if token.value != "|" and token.kind is not Kind.NEWLINE:
                patterns.append(token.value)
            self._at += 1
        self._at += 1
        return Arm(tuple(patterns), self.parse(frozenset({";;", "esac"})))

    def _simple(self) -> Command | None:
        argv: list[str] = []
        assignments: list[str] = []
        line = 0
        while (token := self._peek()) is not None:
            if token.kind is Kind.NEWLINE or token.value in SEPARATORS:
                break
            if token.kind is Kind.OPERATOR:
                redirect = REDIRECT.fullmatch(token.text)
                if redirect is None:
                    break
                self._at += 1
                # A heredoc's delimiter and an `&fd` target were both consumed
                # by the lexer. Skipping a second token here ate the newline
                # after `cat <<EOF`, which merged the next command into this
                # one and reported a script's whole tail as arguments to `cat`.
                if redirect.group("op") in {"<<", "<<-"} or redirect.group("target"):
                    continue
                if (target := self._peek()) is not None and target.kind is Kind.WORD:
                    self._at += 1
                continue
            if not argv and "=" in token.value.split("/")[0] and not token.value.startswith("-"):
                assignments.append(token.value)
            else:
                argv.append(token.value)
            line = line or token.line
            self._at += 1
        if not argv:
            return None
        return Command(tuple(argv), tuple(assignments), line)


def parse(source: str) -> list[Node]:
    """Parse one shell body. Never raises on malformed input."""
    return Parser(tokenize(source)).parse()
