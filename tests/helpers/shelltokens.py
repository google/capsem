"""A tokenizer for the shell subset GitHub `run:` steps are written in.

`shlex` was used line by line, which is wrong for one specific and load-bearing
reason: a shell word may contain a newline. Lexing each physical line alone
raised `ValueError: No closing quotation` on four of the 184 `run:` steps in
this repository, all of them in release.yaml. Accumulating lines until the
quotes happened to balance was tried and is worse -- it stops the crash without
parsing anything, yielding a blob token instead of commands, which looks like
analysis and is not.

So: scan characters, not lines. A newline inside a quotation is part of the
word by construction, and no state has to be guessed.

The grammar, which is the subset a `run:` step actually needs and deliberately
not POSIX shell:

    script      ::= logical_line*
    logical_line::= token* NEWLINE            -- NEWLINE only outside quotes
    token       ::= operator | redirection | word
    operator    ::= '&&' | '||' | ';;' | ';' | '|&' | '|' | '&' | '(' | ')'
    redirection ::= digit* redirect_op ('&' digit+ '-'?)?
    redirect_op ::= '<<<' | '<<' | '>>' | '>&' | '<&' | '>|' | '<>' | '>' | '<'
    word        ::= part+
    part        ::= bare | single | double | expression
    single      ::= "'" not_quote* "'"        -- no escapes inside, per POSIX
    double      ::= '"' (escape | not_dquote)* '"'
    escape      ::= '\\' any
    expression  ::= '${{' any* '}}'           -- GitHub, substituted before bash
    comment     ::= '#' not_newline*          -- only where a word may start

Not modelled, because nothing here asks about them: here-documents, parameter
expansion structure, arithmetic, subshell nesting depth, redirection targets.
A question needing those wants a real bash parser, not this.

What callers rely on: operators survive as their own tokens, so a fail-open
suffix cannot vanish into a word, and one logical line is one tuple.
"""

from __future__ import annotations

import re

_EXPRESSION = re.compile(r"\$\{\{\s*(.*?)\s*\}\}", re.S)

#: Longest first, so `&&` is never read as two `&`.
OPERATORS = ("&&", "||", ";;", "|&", ";", "|", "&", "(", ")")

#: Longest first again: `2>&1` is one redirection, not `2>` then a background
#: `&`. Splitting it was the first bug this tokenizer had.
REDIRECTS = ("<<<", "<<", ">>", ">&", "<&", ">|", "<>", ">", "<")

_OPERATOR_CHARS = frozenset("&|;()")
_REDIRECT_CHARS = frozenset("<>")


class UnterminatedQuote(ValueError):
    """A quotation the script never closes.

    Its own type so a caller decides deliberately rather than catching the bare
    `ValueError` and treating an unreadable step as a clean one -- the same
    fail-open shape these contracts exist to catch.
    """


def tokenize(script: str) -> tuple[tuple[str, ...], ...]:
    """Split `script` into logical lines of tokens.

    Quotes are removed from word text, as a shell would, so
    `test "$X" = success` yields `('test', '$X', '=', 'success')`.
    """
    expressions: dict[str, str] = {}

    def mask(match: re.Match[str]) -> str:
        marker = f"\x00EXPR{len(expressions)}\x00"
        expressions[marker] = "${{ " + " ".join(match.group(1).split()) + " }}"
        return marker

    text = _EXPRESSION.sub(mask, script)

    lines: list[tuple[str, ...]] = []
    tokens: list[str] = []
    word: list[str] = []
    has_word = False  # A word can be legitimately empty: `""` is a token.
    index = 0
    length = len(text)

    def end_word() -> None:
        nonlocal has_word
        if has_word:
            rendered = "".join(word)
            for marker, original in expressions.items():
                rendered = rendered.replace(marker, original)
            tokens.append(rendered)
            word.clear()
            has_word = False

    def end_line() -> None:
        end_word()
        if tokens:
            lines.append(tuple(tokens))
            tokens.clear()

    while index < length:
        char = text[index]

        if char == "\\" and index + 1 < length and text[index + 1] == "\n":
            index += 2  # Line continuation: the newline is not a separator.
            continue

        if char == "\n":
            end_line()
            index += 1
            continue

        if char in " \t":
            end_word()
            index += 1
            continue

        if char == "#" and not has_word:
            while index < length and text[index] != "\n":
                index += 1
            continue

        if char in _REDIRECT_CHARS:
            # A leading file descriptor belongs to the redirection, not to the
            # word before it: `2>&1` is one token. The old lexer emitted
            # ('2', '>&', '1'), which reads as a word, an operator and a word.
            descriptor = ""
            pending = "".join(word)
            if has_word and pending.isdigit():
                descriptor = pending
                word.clear()
                has_word = False
            else:
                end_word()
            operator = next(text[index:][: len(r)] for r in REDIRECTS if text.startswith(r, index))
            index += len(operator)
            target = ""
            if operator.endswith("&"):
                while index < length and text[index].isdigit():
                    target += text[index]
                    index += 1
                if index < length and text[index] == "-":
                    target += "-"
                    index += 1
            tokens.append(descriptor + operator + target)
            continue

        if char in _OPERATOR_CHARS:
            end_word()
            for operator in OPERATORS:
                if text.startswith(operator, index):
                    tokens.append(operator)
                    index += len(operator)
                    break
            else:  # pragma: no cover -- _OPERATOR_CHARS and OPERATORS agree
                raise AssertionError(f"unhandled operator character {char!r}")
            continue

        if char == "$" and text.startswith("$(", index):
            # Command substitution is its own quoting context: the quotes
            # inside `$(sed 's/"//')` do not close a quote outside it. Consume
            # the balanced region verbatim rather than lexing into it -- what
            # it evaluates to is not a question this module answers.
            close = _matching_paren(text, index + 1)
            if close == -1:
                raise UnterminatedQuote("unterminated command substitution:\n" + script)
            word.append(text[index : close + 1])
            has_word = True
            index = close + 1
            continue

        if char == "'":
            close = text.find("'", index + 1)
            if close == -1:
                raise UnterminatedQuote("unterminated single quote:\n" + script)
            word.append(text[index + 1 : close])
            has_word = True
            index = close + 1
            continue

        if char == '"':
            index += 1
            while index < length and text[index] != '"':
                if text.startswith("$(", index):
                    close = _matching_paren(text, index + 1)
                    if close == -1:
                        raise UnterminatedQuote(
                            "unterminated command substitution:\n" + script
                        )
                    word.append(text[index : close + 1])
                    index = close + 1
                    continue
                if text[index] == "\\" and index + 1 < length:
                    following = text[index + 1]
                    if following == "\n":
                        index += 2  # Continuation inside a double quote.
                        continue
                    # Backslash is literal in double quotes except before these.
                    if following in '"\\$`':
                        word.append(following)
                        index += 2
                        continue
                word.append(text[index])
                index += 1
            if index >= length:
                raise UnterminatedQuote("unterminated double quote:\n" + script)
            has_word = True
            index += 1
            continue

        if char == "\\" and index + 1 < length:
            word.append(text[index + 1])
            has_word = True
            index += 2
            continue

        word.append(char)
        has_word = True
        index += 1

    end_line()
    return tuple(lines)


def _matching_paren(text: str, opening: int) -> int:
    """Index of the `)` closing the `(` at `opening`, or -1.

    Quotes inside are skipped so a `)` in a string cannot close the region,
    and nesting is counted so `$(a $(b))` resolves to the outer one.
    """
    depth = 0
    index = opening
    length = len(text)
    while index < length:
        char = text[index]
        if char == "\\" and index + 1 < length:
            index += 2
            continue
        if char in "'\"":
            close = text.find(char, index + 1)
            if close == -1:
                return -1
            index = close + 1
            continue
        if char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
            if depth == 0:
                return index
        index += 1
    return -1
