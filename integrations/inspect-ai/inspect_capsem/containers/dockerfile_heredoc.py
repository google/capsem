"""Pure Dockerfile support from Pierre Tholoniat's bb61fc82d integration."""

from __future__ import annotations

import posixpath
import shlex

from .dockerfile_scan import _scan_dockerfile_instructions


def _shell_single_quote(s: str) -> str:
    return "'" + s.replace("'", "'\"'\"'") + "'"


def _escape_unquoted_heredoc_line(ln: str, *, allow_cmd_subst: bool = True) -> str:
    """Translate one line of an unquoted `<<EOF` body for use inside POSIX `"..."`."""
    out: list[str] = []
    i = 0
    n = len(ln)
    while i < n:
        ch = ln[i]
        if ch == "\\":
            if i + 1 < n and ln[i + 1] in ("$", "`", "\\"):
                out.append(ln[i : i + 2])
                i += 2
                continue
            out.append("\\\\")
            i += 1
            continue
        if ch == '"':
            out.append('\\"')
            i += 1
            continue
        if not allow_cmd_subst:
            if ch == "`":
                out.append("\\`")
                i += 1
                continue
            if ch == "$" and i + 1 < n and ln[i + 1] == "(":
                out.append("\\$(")
                i += 2
                continue
        out.append(ch)
        i += 1
    return "".join(out)


def _format_printf_heredoc_cmd(
    body_lines: list[str], *, is_quoted: bool, allow_cmd_subst: bool = True
) -> str:
    """Render `body_lines` as a single-line POSIX `printf` command without physical newlines."""
    if not body_lines:
        return "printf ''"
    if is_quoted or not any(("$" in ln or "`" in ln or "\\" in ln) for ln in body_lines):
        args = " ".join(_shell_single_quote(ln) for ln in body_lines)
        return f"printf '%s\\n' {args}"
    dq_parts = [
        f'"{_escape_unquoted_heredoc_line(ln, allow_cmd_subst=allow_cmd_subst)}"'
        for ln in body_lines
    ]
    return f"printf '%s\\n' {' '.join(dq_parts)}"


def _lower_single_instruction_heredocs(
    keyword: str,
    header_text: str,
    openers: list[tuple[int, int, bool, bool, str]],
    bodies: list[list[str]],
) -> list[str]:
    """Lower a single `RUN` or `COPY`/`ADD` instruction with BuildKit heredocs to classic `RUN`."""
    if keyword in ("COPY", "ADD"):
        # Strip keyword and heredoc tokens to parse flags (--chmod, --chown) and destination path.
        spans = [(s, e) for s, e, _, _, _ in openers]
        cleaned_chars: list[str] = []
        cursor = 0
        for s, e in spans:
            cleaned_chars.append(header_text[cursor:s])
            cursor = e
        cleaned_chars.append(header_text[cursor:])
        remainder = "".join(cleaned_chars).split(None, 1)
        tail = remainder[1] if len(remainder) > 1 else ""
        try:
            tokens = shlex.split(tail)
        except ValueError:
            tokens = tail.split()
        chmod_val: str | None = None
        chown_val: str | None = None
        pos_args: list[str] = []
        for tok in tokens:
            if tok.startswith("--chmod="):
                chmod_val = tok.split("=", 1)[1]
            elif tok.startswith("--chown="):
                chown_val = tok.split("=", 1)[1]
            elif tok.startswith("--"):
                continue
            else:
                pos_args.append(tok)
        dest = pos_args[-1] if pos_args else "/tmp/heredoc"
        multi = len(openers) > 1 or dest.endswith("/")
        cmds: list[str] = []
        for (_, _, _, is_quoted, delim), body_lines in zip(openers, bodies, strict=True):
            target = posixpath.join(dest.rstrip("/") or "/", delim) if multi else dest
            parent = posixpath.dirname(target) or "/"
            target_q = shlex.quote(target)
            parent_q = shlex.quote(parent)
            printf_cmd = _format_printf_heredoc_cmd(
                body_lines, is_quoted=is_quoted, allow_cmd_subst=False
            )
            step_parts = [f"mkdir -p {parent_q}", f"{printf_cmd} > {target_q}"]
            if chmod_val:
                step_parts.append(f"chmod {shlex.quote(chmod_val)} {target_q}")
            if chown_val:
                step_parts.append(f"chown {shlex.quote(chown_val)} {target_q}")
            cmds.append(" && ".join(step_parts))
        return [f"RUN {' && '.join(cmds)}"]

    # keyword == "RUN"
    # Remove leading "RUN" and any BuildKit "--mount=... / --network=... / --security=..." flags
    after_kw = header_text.split(None, 1)[1] if len(header_text.split(None, 1)) > 1 else ""
    first_start, first_end, _, first_quoted, _ = openers[0]
    kw_offset = len(header_text) - len(after_kw)
    rel_start = first_start - kw_offset
    rel_end = first_end - kw_offset
    before_token = after_kw[:rel_start].strip()
    after_token = after_kw[rel_end:].strip()

    # Strip leading `--flag=...` tokens from before_token
    flag_parts: list[str] = []
    while before_token.startswith("--"):
        parts = before_token.split(None, 1)
        flag_parts.append(parts[0])
        before_token = parts[1].strip() if len(parts) > 1 else ""

    body_lines = bodies[0]
    if not before_token and not after_token and len(openers) == 1:
        # Bare `RUN <<EOF` — always single-quote the script body so the script's own
        # interpreter expands variables/loops, and execute from a temp file without forcing -e.
        if body_lines and body_lines[0].startswith("#!"):
            shebang = body_lines[0][2:].strip() or "/bin/sh"
            script_lines = body_lines[1:]
            printf_cmd = _format_printf_heredoc_cmd(script_lines, is_quoted=True)
            return [
                f'RUN _f=$(mktemp) && {printf_cmd} > "$_f" && chmod +x "$_f" '
                f'&& {{ {shebang} "$_f"; _rc=$?; rm -f "$_f"; exit $_rc; }}'
            ]
        printf_cmd = _format_printf_heredoc_cmd(body_lines, is_quoted=True)
        return [
            f'RUN _f=$(mktemp) && {printf_cmd} > "$_f" '
            f'&& {{ sh "$_f"; _rc=$?; rm -f "$_f"; exit $_rc; }}'
        ]

    if len(openers) == 1:
        cmd_rest = f"{before_token} {after_token}".strip()
        printf_cmd = _format_printf_heredoc_cmd(body_lines, is_quoted=first_quoted)
        return [f"RUN {printf_cmd} | {{ {cmd_rest}; }}"]

    # Multiple heredocs in a single RUN instruction: materialize temp files and substitute paths
    rebuilt: list[str] = []
    setup_parts: list[str] = []
    cleanup_paths: list[str] = []
    cursor = kw_offset
    for idx, ((s, e, _, is_quoted, _), b_lines) in enumerate(zip(openers, bodies, strict=True)):
        tmp_path = f"/tmp/.capsem_hd_{idx}"
        cleanup_paths.append(tmp_path)
        printf_cmd = _format_printf_heredoc_cmd(b_lines, is_quoted=is_quoted)
        setup_parts.append(f"{printf_cmd} > {tmp_path}")
        rebuilt.append(header_text[cursor:s])
        rebuilt.append(f"< {tmp_path}")
        cursor = e
    rebuilt.append(header_text[cursor:])
    cmd_str = "".join(rebuilt).strip()
    while cmd_str.startswith("--"):
        parts = cmd_str.split(None, 1)
        cmd_str = parts[1].strip() if len(parts) > 1 else ""
    rm_cmd = f"rm -f {' '.join(cleanup_paths)}"
    return [f"RUN {' && '.join(setup_parts)} && {{ {cmd_str}; rc=$?; {rm_cmd}; exit $rc; }}"]


def lower_dockerfile_heredocs(text: str) -> str:
    """Lower BuildKit `RUN <<EOF` and `COPY <<EOF <dest>` heredocs for classic `docker build`.

    Capsem's guest `dockerd` runs with `"features": {"buildkit": false}` because BuildKit requires
    `mount(2)` inside user/mount namespaces. Classic `docker build` splits Dockerfiles by physical
    newline first and fails on multi-line BuildKit heredocs (`Unknown instruction` or `COPY requires
    at least two arguments`). This function lowers `RUN` and `COPY`/`ADD` heredoc instructions into
    equivalent single-line `RUN printf ...` instructions while leaving non-heredoc lines untouched.
    """
    out: list[str] = []
    changed = False
    for keyword, cur, raw, openers, bodies in _scan_dockerfile_instructions(text.splitlines()):
        if openers:
            changed = True
            out.extend(_lower_single_instruction_heredocs(keyword, cur, openers, bodies))
        else:
            out.extend(raw)
    if not changed:
        return text
    trailing_nl = "\n" if text.endswith("\n") else ""
    return "\n".join(out) + trailing_nl
