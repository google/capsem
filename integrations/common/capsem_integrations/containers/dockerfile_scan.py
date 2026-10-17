"""Pure Dockerfile support from Pierre Tholoniat's bb61fc82d integration."""

from __future__ import annotations

import re

_ESCAPE_DIRECTIVE_RE = re.compile(r"^#\s*escape\s*=\s*([\\`])\s*$", re.IGNORECASE)

_HEREDOC_TOKEN_RE = re.compile(r"<<(-?)\s*([\"']?)([A-Za-z_]\w*)\2")


def _find_heredoc_openers_detailed(
    text: str, escape_char: str = "\\"
) -> list[tuple[int, int, bool, bool, str]]:
    """Return `(start, end, strip_tabs, is_quoted, delimiter)` for heredoc openers outside quotes in `text`."""
    openers: list[tuple[int, int, bool, bool, str]] = []
    quote: str | None = None
    arith_depth = 0
    pos = 0
    n = len(text)
    while pos < n:
        ch = text[pos]
        if quote == "'":
            if ch == "'":
                quote = None
            pos += 1
            continue
        if quote == '"':
            if ch == escape_char and pos + 1 < n:
                pos += 2
                continue
            if ch == '"':
                quote = None
            pos += 1
            continue
        if ch == escape_char and pos + 1 < n:
            pos += 2
            continue
        if ch in ("'", '"'):
            quote = ch
            pos += 1
            continue
        if text.startswith("$((", pos):
            arith_depth += 1
            pos += 3
            continue
        if text.startswith("((", pos) and (
            arith_depth > 0 or pos == 0 or text[pos - 1] in " \t\n;|&({"
        ):
            arith_depth += 1
            pos += 2
            continue
        if arith_depth > 0:
            if text.startswith("))", pos):
                arith_depth = max(0, arith_depth - 1)
                pos += 2
                continue
            if ch == "(":
                arith_depth += 1
            elif ch == ")":
                arith_depth = max(0, arith_depth - 1)
            pos += 1
            continue
        if ch == "<" and pos + 1 < n and text[pos + 1] == "<":
            if pos + 2 < n and text[pos + 2] == "<":
                while pos < n and text[pos] == "<":
                    pos += 1
                continue
            prev = text[pos - 1] if pos > 0 else " "
            is_bitshift = prev.isalpha() or prev == "_"
            if not is_bitshift and prev.isdigit():
                k = pos - 1
                while k > 0 and (text[k - 1].isalnum() or text[k - 1] == "_"):
                    k -= 1
                before_num = text[k - 1] if k > 0 else " "
                if (pos - k) > 1 or before_num in "([+-*/%&|^~<>=":
                    is_bitshift = True
            if not is_bitshift:
                m = _HEREDOC_TOKEN_RE.match(text, pos)
                if m is not None:
                    openers.append(
                        (m.start(), m.end(), m.group(1) == "-", bool(m.group(2)), m.group(3))
                    )
                    pos = m.end()
                    continue
        pos += 1
    return openers


def _scan_dockerfile_instructions(
    lines: list[str],
) -> list[tuple[str, str, list[str], list[tuple[int, int, bool, bool, str]], list[list[str]]]]:
    """Scan a Dockerfile into `(keyword, joined_text, physical_lines, heredoc_openers, heredoc_bodies)`."""
    out: list[
        tuple[str, str, list[str], list[tuple[int, int, bool, bool, str]], list[list[str]]]
    ] = []
    escape_char = "\\"
    in_header_directives = True
    i = 0
    while i < len(lines):
        start, text = i, lines[i]
        i += 1
        stripped = text.strip()
        if not stripped:
            in_header_directives = False
            out.append(("", text, lines[start:i], [], []))
            continue
        if stripped.startswith("#"):
            if in_header_directives:
                m_esc = _ESCAPE_DIRECTIVE_RE.match(stripped)
                if m_esc is not None:
                    escape_char = m_esc.group(1)
                elif "=" not in stripped:
                    in_header_directives = False
            out.append(("", text, lines[start:i], [], []))
            continue
        in_header_directives = False
        while text.rstrip().endswith(escape_char) and i < len(lines):
            next_line = lines[i]
            i += 1
            if next_line.lstrip().startswith("#"):
                continue
            text = text.rstrip()[:-1] + " " + next_line
        keyword = stripped.split(None, 1)[0].upper()
        openers: list[tuple[int, int, bool, bool, str]] = []
        bodies: list[list[str]] = []
        if keyword in ("RUN", "COPY", "ADD"):
            openers = _find_heredoc_openers_detailed(text, escape_char)
            for _, _, strip_tabs, _, word in openers:
                body_lines: list[str] = []
                terminated = False
                while i < len(lines):
                    raw_body = lines[i]
                    i += 1
                    candidate = raw_body.lstrip("\t") if strip_tabs else raw_body
                    if candidate.rstrip(" \t\r") == word:
                        terminated = True
                        break
                    body_lines.append(candidate.rstrip("\r"))
                if not terminated:
                    msg = (
                        f"Unterminated heredoc <<{word} in Dockerfile instruction "
                        f"at line {start + 1}"
                    )
                    raise ValueError(msg)
                bodies.append(body_lines)
        out.append((keyword, text, lines[start:i], openers, bodies))
    return out


def _dockerfile_instructions(lines: list[str]) -> list[tuple[str, str, list[str]]]:
    """Split a Dockerfile into (KEYWORD, joined text, physical lines) per instruction.

    Continuation lines are joined and heredoc bodies are consumed, so a body line starting
    with `from ` is never read as an instruction. Blank and comment lines get keyword "".
    """
    return [(kw, text, raw) for kw, text, raw, _, _ in _scan_dockerfile_instructions(lines)]
