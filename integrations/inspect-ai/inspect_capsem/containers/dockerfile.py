"""Dockerfile heredoc lowering, Capsem MITM CA injection, and WORKDIR resolution."""

from __future__ import annotations

import logging
import posixpath
import re
import shlex
from pathlib import Path

from inspect_capsem.containers import nss_trust as _nss_trust

logger = logging.getLogger(__name__)

_BUILD_DIR = "/tmp/capsem-build"
# The CA-patched Dockerfile goes beside the context, so the original stays untouched.
_CA_DF_DIR = "/tmp/capsem-build-ca"
_CA_READY = "CAPSEM_CA_READY"
_VM_CA = "/usr/local/share/ca-certificates/capsem-ca.crt"
# In-image paths. The CA sits where Debian's update-ca-certificates merges it into the
# image's own bundle; the merged bundle is a separate file the env vars point at, so
# the image's bundle is never replaced.
_IMAGE_CA = "/usr/local/share/ca-certificates/capsem-ca.crt"
_IMAGE_CA_BUNDLE = "/usr/local/share/capsem/ca-bundle.crt"
_CA_PREP_CMD = f"""set -e
[ -f {_VM_CA} ] || {{ echo NO_CAPSEM_CA; exit 0; }}
cd {_BUILD_DIR}
cp {_VM_CA} .capsem-ca.crt
cat /etc/ssl/certs/ca-certificates.crt {_VM_CA} > .capsem-ca-bundle.crt
chmod 644 .capsem-ca.crt .capsem-ca-bundle.crt
if [ -f .dockerignore ]; then printf '\\n!.capsem-ca.crt\\n!.capsem-ca-bundle.crt\\n' >> .dockerignore; fi
sha256sum {_VM_CA} 2>/dev/null | awk '{{print "{_CA_READY}:" $1}}' || echo {_CA_READY}"""
# Env vars pointing TLS clients at the merged bundle; Dockerfile stages get them as an
# ENV line, image-only services as `docker run -e`.
_CA_ENV = {
    "SSL_CERT_FILE": _IMAGE_CA_BUNDLE,
    "REQUESTS_CA_BUNDLE": _IMAGE_CA_BUNDLE,
    "PIP_CERT": _IMAGE_CA_BUNDLE,
    "CURL_CA_BUNDLE": _IMAGE_CA_BUNDLE,
    "GIT_SSL_CAINFO": _IMAGE_CA_BUNDLE,
    "NODE_EXTRA_CA_CERTS": _IMAGE_CA,
    "UV_NATIVE_TLS": "1",
}
_CA_STAGE_LINES = (
    f"COPY .capsem-ca.crt {_IMAGE_CA}",
    f"COPY .capsem-ca-bundle.crt {_IMAGE_CA_BUNDLE}",
    "ENV " + " ".join(f"{k}={v}" for k, v in _CA_ENV.items()),
)
# Merges the CA into the image's system store (appending, never replacing), for tools
# that ignore the env vars: apt's GnuTLS, Java via ca-certificates-java. Best-effort: a
# no-op without the tool (RHEL's update-ca-trust, distroless) or without root/sudo.
_CA_UPDATE_LINE = (
    "RUN command -v update-ca-certificates >/dev/null 2>&1 "
    '&& { [ "$(id -u)" = 0 ] && update-ca-certificates >/dev/null 2>&1 '
    "|| sudo -n update-ca-certificates >/dev/null 2>&1; } || true"
)
_ESCAPE_DIRECTIVE_RE = re.compile(r"^#\s*escape\s*=\s*([\\`])\s*$", re.IGNORECASE)
# BuildKit heredoc opener (`<<EOF`, `<<-"EOF"`).
_HEREDOC_TOKEN_RE = re.compile(r"<<(-?)\s*([\"']?)([A-Za-z_]\w*)\2")


def _extract_ca_fingerprint(stdout: str) -> str:
    """Return the hex SHA-256 fingerprint emitted after `CAPSEM_CA_READY:`, if present."""
    for line in stdout.splitlines():
        s = line.strip()
        if s.startswith(f"{_CA_READY}:"):
            return s.split(":", 1)[1].strip()
    return ""


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


def _stage_runs_shell(rest: list[tuple[str, str, list[str]]]) -> bool:
    """Whether the stage starting at `rest` has a shell-form RUN (so /bin/sh exists)."""
    for keyword, text, _ in rest:
        if keyword == "FROM":
            return False
        if keyword == "RUN":
            args = [a for a in text.split()[1:] if not a.startswith("--")]
            if args and not args[0].startswith("["):
                return True
    return False


def _stage_has_any_run(rest: list[tuple[str, str, list[str]]]) -> bool:
    """Whether the stage starting at `rest` has any RUN instruction (shell or exec form)."""
    for keyword, _, _ in rest:
        if keyword == "FROM":
            return False
        if keyword == "RUN":
            return True
    return False


# The VM's dockerd runs with HOME=/ and `docker exec` leaks it into the container
# instead of the exec user's passwd home, so Playwright looks under /.cache.
_CONTAINER_HOME_FIX = (
    'if [ "${HOME:-/}" = / ]; then '
    "_h=$(awk -F: -v u=\"$(id -u)\" '$3 == u {print $6; exit}' /etc/passwd 2>/dev/null); "
    'if [ -n "$_h" ]; then export HOME="$_h"; fi; unset _h; '
    "fi; "
)
_NSS_TRUST_SRC = Path(_nss_trust.__file__).read_text(encoding="utf-8")
# As root: the CA into the system store (merged by update-ca-certificates where it
# exists) plus a separate bundle for _CA_ENV built from the image's own bundle.
_CA_SYSTEM_SCRIPT = (
    f"mkdir -p {posixpath.dirname(_IMAGE_CA)} {posixpath.dirname(_IMAGE_CA_BUNDLE)} && "
    f"cat > {_IMAGE_CA} && "
    f"{{ cat /etc/ssl/certs/ca-certificates.crt 2>/dev/null; cat {_IMAGE_CA}; }} "
    f"> {_IMAGE_CA_BUNDLE} && chmod 644 {_IMAGE_CA} {_IMAGE_CA_BUNDLE} && "
    "{ ! command -v update-ca-certificates >/dev/null || "
    "update-ca-certificates >/dev/null 2>&1 || true; }"
)
# As the default user: Chromium on Linux only trusts extra roots from the NSS user DB.
_CA_NSS_SCRIPT = (
    f"{_CONTAINER_HOME_FIX}"
    "if command -v certutil >/dev/null; then "
    'db="sql:$HOME/.pki/nssdb"; mkdir -p "$HOME/.pki/nssdb"; '
    'certutil -d "$db" -L >/dev/null 2>&1 || certutil -d "$db" -N --empty-password; '
    f'certutil -d "$db" -A -n capsem-ca -t C,, -i {_IMAGE_CA}; '
    "elif command -v python3 >/dev/null && "
    "python3 -c 'import ctypes.util as u, sys; sys.exit(not u.find_library(\"nss3\"))' "
    "2>/dev/null; then "
    f"python3 -c {shlex.quote(_NSS_TRUST_SRC)} {_IMAGE_CA}; "
    "fi"
)


def _ca_inject_command(cid: str) -> str:
    """VM-side command that puts the Capsem MITM CA into container `cid`'s system store."""
    script = shlex.quote(_CA_SYSTEM_SCRIPT)
    return f"docker exec -i -u 0 {shlex.quote(cid)} sh -c {script} < {_VM_CA}"


def _ca_nss_command(cid: str, *, user: str | None = None) -> str:
    """VM-side command that adds the CA to the container's default user's NSS DB."""
    user_flag = f"-u {shlex.quote(user)} " if user else ""
    return f"docker exec {user_flag}{shlex.quote(cid)} sh -c {shlex.quote(_CA_NSS_SCRIPT)}"


def is_root_user_spec(user: str | None) -> bool:
    """Return True when `user` is unset, empty, or resolves to root (`root`, `0`, `0:0`, `root:root`)."""
    if user is None:
        return True
    head = user.strip().split(":", 1)[0].strip().lower()
    return head in ("", "root", "0")


def patch_dockerfile_for_capsem_ca(text: str) -> str:
    """Add the Capsem MITM CA to every stage that can make TLS calls during the build.

    Build stages need it as much as the final one (they fetch too). Stages `FROM scratch`
    are skipped (no RUN can execute there), as are stages built from an earlier stage,
    which inherit the CA files. If an earlier stage had no shell-form RUN, a derived stage
    that *does* run a shell still emits `update-ca-certificates` (wrapping in `USER root`
    and restoring the inherited user if the parent stage ended as non-root). And if a stage
    switches back to `USER root` / `USER 0` before a `RUN` (shell or exec form),
    `update-ca-certificates` is emitted after the `USER` switch so the system store is
    updated as root.
    """
    instructions = _dockerfile_instructions(text.splitlines())
    out: list[str] = []
    stages: set[str] = set()
    updated_stages: set[str] = set()
    root_updated_stages: set[str] = set()
    stage_end_user: dict[str, str] = {}
    current_alias: str | None = None
    current_has_ca_files = False
    current_runs_shell = False
    current_updated_as_root = False
    current_user = ""
    for idx, (keyword, joined, raw) in enumerate(instructions):
        out.extend(raw)
        if keyword == "USER":
            parts = joined.split(None, 1)
            user_spec = parts[1].strip() if len(parts) > 1 else ""
            current_user = user_spec
            if current_alias:
                stage_end_user[current_alias] = current_user
            if (
                user_spec
                and is_root_user_spec(user_spec)
                and current_has_ca_files
                and not current_updated_as_root
                and (current_runs_shell or _stage_has_any_run(instructions[idx + 1 :]))
            ):
                if not out or out[-1] != _CA_UPDATE_LINE:
                    out.append(_CA_UPDATE_LINE)
                current_updated_as_root = True
                if current_alias:
                    updated_stages.add(current_alias)
                    root_updated_stages.add(current_alias)
            continue
        if keyword != "FROM":
            continue
        args = [a for a in joined.split()[1:] if not a.startswith("--")]
        base = args[0].lower() if args else "scratch"
        alias = args[2].lower() if len(args) >= 3 and args[1].lower() == "as" else None
        current_alias = alias
        current_runs_shell = _stage_runs_shell(instructions[idx + 1 :])
        has_ca_files = base in stages
        has_ca_update = base in updated_stages
        current_updated_as_root = base in root_updated_stages
        current_user = stage_end_user.get(base, "")
        if base != "scratch":
            if not has_ca_files:
                out.extend(_CA_STAGE_LINES)
                has_ca_files = True
            if has_ca_files and not has_ca_update and current_runs_shell:
                if current_user and not is_root_user_spec(current_user):
                    out.append("USER root")
                    out.append(_CA_UPDATE_LINE)
                    out.append(f"USER {current_user}")
                    has_ca_update = True
                    current_updated_as_root = True
                else:
                    out.append(_CA_UPDATE_LINE)
                    has_ca_update = True
                    if current_user and is_root_user_spec(current_user):
                        current_updated_as_root = True
        current_has_ca_files = has_ca_files
        if alias:
            stage_end_user[alias] = current_user
            if has_ca_files:
                stages.add(alias)
            if has_ca_update:
                updated_stages.add(alias)
            if current_updated_as_root:
                root_updated_stages.add(alias)
    return "\n".join(out) + "\n"


_DOCKERFILE_VAR_RE = re.compile(r"\$\{([A-Za-z_]\w*)(?::(-|\+)([^}]*))?\}|\$([A-Za-z_]\w*)")


def _expand_dockerfile_vars(value: str, env_vars: dict[str, str]) -> str:
    def _repl(match: re.Match[str]) -> str:
        braced_name, op, default_val, bare_name = match.groups()
        name = braced_name or bare_name
        val = env_vars.get(name)
        if op == "-":
            return val if val else (default_val or "")
        if op == "+":
            return (default_val or "") if val else ""
        if val is not None:
            return val
        return match.group(0)

    return _DOCKERFILE_VAR_RE.sub(_repl, value)


def _strip_optional_quotes(value: str) -> str:
    val = value.strip()
    if len(val) >= 2 and val[0] == val[-1] and val[0] in ("'", '"'):
        return val[1:-1]
    return val


def _update_dockerfile_vars(
    keyword: str, joined: str, target_vars: dict[str, str], fallback_vars: dict[str, str]
) -> None:
    parts = joined.split(None, 1)
    if len(parts) < 2:
        return
    rest = parts[1].strip()
    if not rest:
        return
    first_token = rest.split(None, 1)[0]
    if keyword == "ENV" and "=" not in first_token:
        kv = rest.split(None, 1)
        if len(kv) == 2:
            target_vars[kv[0]] = _expand_dockerfile_vars(_strip_optional_quotes(kv[1]), target_vars)
        return
    try:
        tokens = shlex.split(rest)
    except ValueError:
        tokens = rest.split()
    for tok in tokens:
        if "=" in tok:
            k, _, v = tok.partition("=")
            if k:
                target_vars[k] = _expand_dockerfile_vars(_strip_optional_quotes(v), target_vars)
        elif keyword == "ARG" and tok in fallback_vars and tok not in target_vars:
            target_vars[tok] = fallback_vars[tok]


def _extract_dockerfile_metadata(text: str) -> tuple[str | None, str | None]:
    """Return `(workdir, user)` for the final stage of a Dockerfile (`None` when unset/root)."""
    instructions = _dockerfile_instructions(text.splitlines())
    global_vars: dict[str, str] = {}
    stage_workdirs: dict[str, str | None] = {}
    stage_users: dict[str, str | None] = {}
    stage_vars: dict[str, dict[str, str]] = {}
    in_stage = False
    current_alias: str | None = None
    current_workdir: str | None = None
    current_user: str | None = None
    current_vars: dict[str, str] = {}

    for keyword, joined, _ in instructions:
        if keyword == "FROM":
            args = [a for a in joined.split()[1:] if not a.startswith("--")]
            raw_base = _expand_dockerfile_vars(args[0], global_vars) if args else "scratch"
            base = raw_base.lower()
            alias = args[2].lower() if len(args) >= 3 and args[1].lower() == "as" else None
            in_stage = True
            current_alias = alias
            current_workdir = stage_workdirs.get(base)
            current_user = stage_users.get(base)
            current_vars = dict(global_vars)
            if base in stage_vars:
                current_vars.update(stage_vars[base])
            if alias:
                stage_workdirs[alias] = current_workdir
                stage_users[alias] = current_user
                stage_vars[alias] = current_vars
        elif keyword in ("ARG", "ENV"):
            if in_stage:
                _update_dockerfile_vars(keyword, joined, current_vars, global_vars)
                if current_alias:
                    stage_vars[current_alias] = current_vars
            elif keyword == "ARG":
                _update_dockerfile_vars(keyword, joined, global_vars, global_vars)
        elif keyword == "WORKDIR" and in_stage:
            parts = joined.split(None, 1)
            if len(parts) > 1:
                raw_wd = _strip_optional_quotes(parts[1])
                expanded = _expand_dockerfile_vars(raw_wd, current_vars).strip()
                if expanded and "$" not in expanded:
                    if posixpath.isabs(expanded):
                        current_workdir = posixpath.normpath(expanded)
                    else:
                        current_workdir = posixpath.normpath(
                            posixpath.join(current_workdir or "/", expanded)
                        )
                    if current_alias:
                        stage_workdirs[current_alias] = current_workdir
        elif keyword == "USER" and in_stage:
            parts = joined.split(None, 1)
            if len(parts) > 1:
                raw_user = _strip_optional_quotes(parts[1])
                expanded_user = _expand_dockerfile_vars(raw_user, current_vars).strip()
                if expanded_user and "$" not in expanded_user:
                    current_user = None if is_root_user_spec(expanded_user) else expanded_user
                    if current_alias:
                        stage_users[current_alias] = current_user
    return current_workdir, current_user


def _read_dockerfile_text(dockerfile: str | Path | None) -> str | None:
    if not dockerfile:
        return None
    df_path = Path(dockerfile)
    if not df_path.is_file():
        return None
    try:
        return df_path.read_text(encoding="utf-8", errors="replace")
    except Exception:
        logger.debug("Failed reading Dockerfile %s", df_path, exc_info=True)
        return None


def resolve_dockerfile_defaults(dockerfile: str | Path | None) -> dict[str, str]:
    """Return `{'working_dir': ..., 'user': ...}` defaults extracted from `dockerfile`."""
    text = _read_dockerfile_text(dockerfile)
    if text is None:
        return {}
    workdir, user = _extract_dockerfile_metadata(text)
    defaults: dict[str, str] = {}
    if workdir:
        defaults["working_dir"] = workdir
    if user:
        defaults["user"] = user
    return defaults
