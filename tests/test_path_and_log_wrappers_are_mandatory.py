"""Capsem paths and log streams are reached through their wrapper, never by hand.

Two rules that exist because both were broken the same way -- a rule living in
a caller instead of a function:

**Environment.** `CAPSEM_RUN_DIR` and `CAPSEM_ASSETS_DIR` each take precedence
over the value derived from `CAPSEM_HOME`. A fixture that sets the home alone
leaves production code reading whichever directories the caller exported, and
`just test-clean` exports them. That passes in a bare shell and fails only inside the
gate. `paths::CapsemPathsGuard::redirect(root)` sets all three together, so
setting one and forgetting the others is not expressible.

**Logs.** `<run>/service.log` names a daily-rotated stream, not a file. Opening
it directly returns nothing, which is how `/service-logs` came to report an
empty log for a service that was writing normally.
`telemetry::read_log_tail` resolves the stream and tails it.

A wrapper nobody is obliged to use is a suggestion. These make it the only way.
"""

from __future__ import annotations

import ast
import re
from pathlib import Path

import pytest
from helpers import embedded_shell

PROJECT_ROOT = Path(__file__).resolve().parent.parent
CRATES = PROJECT_ROOT / "crates"

MANAGED_VARS = ("CAPSEM_HOME", "CAPSEM_RUN_DIR", "CAPSEM_ASSETS_DIR")

# Sets a managed variable directly instead of going through the guard.
RAW_SET = re.compile(
    r"""(?:std::env::)?set_var\(\s*["']({})["']""".format("|".join(MANAGED_VARS))
)

# Owns the rule, so it is the one place allowed to implement it.
ENV_OWNER = "paths.rs"
LOG_OWNER = "telemetry.rs"

# Streams whose writer rotates, so the bare name holds only the newest slice.
# Derived from the writers, not from what the readers happen to do: the daemon
# streams go through `tracing-appender`'s daily rotation, and `serial.log` goes
# through `telemetry::CappedLogWriter` in both hypervisor backends.
#
# `pty.log` and `process.log` are deliberately absent. `pty.log` is a binary
# transcript read as bytes and base64-encoded -- routing it through a `String`
# reader would corrupt it -- and `process.log` is a plain appended file. Listing
# a stream here is a claim about its writer; adding one to silence a reader
# would invert the rule.
ROTATING_STREAMS = ("service", "gateway", "mcp", "tray", "serial")

# Reads a file's whole contents directly.
DIRECT_READ = re.compile(r"(File::open|fs::read\b|fs::read_to_string|read_to_string\()")
# Every log path, not just today's rotated daemon streams. A per-session
# serial.log on a persistent VM runs for weeks with no cap, so "it does not
# rotate yet" describes the gap rather than justifying it. Reading through the
# stream reader is correct whether or not a given stream has rotated: it
# returns the single file when there is one.
LOG_STREAM_PATH = re.compile(r"""\.join\(\s*(?:&?format!\()?["'][a-z_]+\.log["']""")


def _rust_sources() -> list[Path]:
    return [p for p in sorted(CRATES.rglob("*.rs")) if p.is_file()]


def test_managed_path_variables_are_set_only_through_the_guard() -> None:
    sources = _rust_sources()
    assert len(sources) > 50, "scanned too few Rust files to trust this guard"

    offenders = []
    for path in sources:
        if path.name == ENV_OWNER:
            continue
        text = path.read_text(encoding="utf-8", errors="ignore")
        for match in RAW_SET.finditer(text):
            line = text[: match.start()].count("\n") + 1
            offenders.append(
                f"{path.relative_to(PROJECT_ROOT)}:{line} sets {match.group(1)} by hand"
            )

    assert not offenders, (
        "set Capsem path variables through paths::CapsemPathsGuard::redirect(root), "
        "which sets all three together -- setting one leaves the others inherited "
        "from the caller, which is invisible locally and breaks in the gate:\n  "
        + "\n  ".join(offenders)
    )


def test_log_streams_are_read_through_the_stream_reader() -> None:
    """A log path may be built and written anywhere; it may not be *read* raw.

    The binding and the read must meet on the same value. Checking whether a
    file merely contains both shapes conflates writing a serial log with
    reading a rotated stream, and flags every file that also happens to read a
    manifest.
    """
    sources = _rust_sources()
    assert len(sources) > 50, "scanned too few Rust files to trust this guard"

    # `let <name> = ... .join("<something>.log")`, or a path handed back by a
    # helper whose entire purpose is to name a host log stream. `handle_host_logs`
    # took its path from `triage::host_log_path` and opened it by hand, which the
    # literal `.join` shape could not see.
    streams = "|".join(ROTATING_STREAMS)
    binding = re.compile(
        r"let\s+(?:mut\s+)?(\w+)\s*(?::[^=]+)?=\s*[^;]*?(?:"
        rf"""\.join\(\s*(?:&?format!\()?["'](?:{streams})\.log["']"""
        r"|host_log_path\("
        r"|latest_app_log\("
        r")[^;]*;",
        re.S,
    )

    offenders = []
    for path in sources:
        if path.name == LOG_OWNER:
            continue
        text = path.read_text(encoding="utf-8", errors="ignore")
        # Per function body, for both the exemption and the binding. A file-level
        # exemption is why this guard stayed quiet while `/host-logs/{name}`
        # returned an empty log for a service that was writing normally:
        # `main.rs` reads one stream through the reader and read another by
        # hand, and one correct call bought silence for the whole file.
        for body in re.split(r"\n(?=\s*(?:pub\s+)?(?:async\s+)?fn\s)", text):
            if "read_log_tail" in body or "log_stream_files" in body:
                continue
            for match in binding.finditer(body):
                name = match.group(1)
                raw_read = re.search(
                    rf"(?:File::open|fs::read|fs::read_to_string|fs::metadata)"
                    rf"\(\s*&?{re.escape(name)}\b",
                    body,
                )
                if raw_read:
                    line = text[: text.index(body)].count("\n") + body[
                        : raw_read.start()
                    ].count("\n") + 1
                    offenders.append(
                        f"{path.relative_to(PROJECT_ROOT)}:{line} reads log path "
                        f"`{name}` directly"
                    )

    assert not offenders, (
        "these open a log path as a file, which returns nothing once that "
        "stream rotates; read through telemetry::read_log_tail:\n  "
        + "\n  ".join(offenders)
    )


# ---------------------------------------------------------------------------
# Python and shell read the same rotated streams as Rust, and were not covered
# until an ironbank ledger test asserted on an empty `service.log`.
#
# This half used to be five regexes. Each was written after a failure, and
# each had a hole found by shipping the bug it was meant to catch: `grep` was
# absent from the read verbs; a function returning the path was not a binding;
# `[^\n|]*?` meant a command wrapped for line length was invisible. Regexes
# fail this way because shell syntax -- quoting, spacing, continuation,
# comments -- is exactly what they have to re-describe every time.
#
# So the shell is tokenised and the Python is parsed. A command is a verb and
# its words however it was written, and a name that denotes a stream path is
# followed whether it was bound by `=`, by a function, or by an assignment
# statement.
# ---------------------------------------------------------------------------

#: Reading is reading. The point is not which tool performs it, which is what
#: a hand-listed set of three verbs kept getting wrong.
READ_VERBS = frozenset(
    {"tail", "cat", "head", "grep", "egrep", "fgrep", "awk", "sed", "less", "more", "wc"}
)


def _is_stream_path(word: str) -> bool:
    """A word naming a rotated stream file, rather than mentioning its name.

    A separator is what distinguishes the two: `assert "service.log" in spawn`
    checks how the service is *configured*, which is right and must keep the
    bare name.
    """
    if "*" in word:
        return False  # globbed: this is reading the stream, which is the ask
    return any(f"/{name}.log" in word for name in ROTATING_STREAMS)


def _denoting_names(shell: str) -> set[str]:
    """Names that stand for a stream path: variables and functions alike.

    A function returning the path is a binding. Not modelling that is what let
    `service_log() {{ echo ".../service.log"; }}` through.
    """
    names = set()
    for command in embedded_shell.commands(shell):
        for word in command:
            # `SERVICE_LOG=$HOME/.capsem/run/service.log`
            if "=" in word:
                name, _, value = word.partition("=")
                if name.isidentifier() and _is_stream_path(value):
                    names.add(name)
    for name, body in embedded_shell.function_bodies(shell).items():
        if any(_is_stream_path(word) for command in embedded_shell.commands(body) for word in command):
            names.add(name)
    return names


def _references(word: str, names: set[str]) -> bool:
    """`$NAME`, `${NAME}` or `$(name)` -- the three ways to spell the read."""
    return any(
        spelling in word
        for name in names
        for spelling in (f"${name}", f"${{{name}}}", f"$({name})")
    )


#: Words that precede a command without being one. `if grep ...` is a grep,
#: and a verb-position check that does not know this sees `if`.
SHELL_KEYWORDS = frozenset({"if", "elif", "while", "until", "then", "do", "else", "!", "time"})


def _verb(command: list[str]) -> str | None:
    for word in command:
        if word not in SHELL_KEYWORDS:
            return word
    return None


def _shell_offenders(shell: str, label: str) -> list[str]:
    names = _denoting_names(shell)
    found = []
    for command in embedded_shell.commands(shell):
        if _verb(command) not in READ_VERBS:
            continue
        for word in command[1:]:
            if _is_stream_path(word):
                found.append(f"{label}: `{' '.join(command)}` reads a rotated stream by name")
                break
            if _references(word, names):
                found.append(f"{label}: `{' '.join(command)}` reads a stream path indirectly")
                break
    return found


class _PythonStreamReads(ast.NodeVisitor):
    """Which expressions open a stream path, following simple bindings.

    `ast` rather than a regex because the read and the binding are routinely
    two hundred lines apart -- `self._log_path` was set in a constructor and
    read in a method, and the single-expression pattern never saw it.
    """

    def __init__(self) -> None:
        self.bound: set[str] = set()
        self.authored: frozenset[str] = frozenset()
        self.reads: list[tuple[frozenset[str], str]] = []

    @staticmethod
    def _streams_named(node: ast.AST) -> frozenset[str]:
        """Which rotated streams this path expression names, if any."""
        return frozenset(
            name
            for inner in ast.walk(node)
            if isinstance(inner, ast.Constant) and isinstance(inner.value, str)
            for name in ROTATING_STREAMS
            if inner.value == f"{name}.log"
        )

    def visit_Assign(self, node: ast.Assign) -> None:
        if self._streams_named(node.value):
            for target in node.targets:
                self.bound.add(ast.unparse(target))
        self.generic_visit(node)

    def visit_Call(self, node: ast.Call) -> None:
        method = node.func
        if isinstance(method, ast.Attribute):
            owner = ast.unparse(method.value)
            if method.attr in {"write_text", "write_bytes"}:
                # A stream this module wrote is a fixture: it knows exactly
                # what is in it. Keyed on the stream rather than the
                # expression, because the write and the read are routinely
                # different paths -- one test writes `run_dir/vm/serial.log`
                # and reads back the copy under `preserved/`.
                self.authored |= self._streams_named(method.value)
            elif method.attr in {"read_text", "read_bytes", "open"}:
                named = self._streams_named(method.value)
                if named or owner in self.bound:
                    self.reads.append((named, ast.unparse(node)))
        self.generic_visit(node)

    @property
    def offenders(self) -> list[str]:
        return [read for named, read in self.reads if not named or not (named <= self.authored)]


def stream_offenders(text: str, label: str = "<text>") -> list[str]:
    """Every rotated-stream read in one file, however it is spelled.

    Extracted so the guard can be run against source that is not in the tree
    -- above all the version that shipped the bug it exists to prevent. A
    guard only exercised on a tree that already passes is a guard nobody has
    watched fail.
    """
    offenders: list[str] = []
    try:
        tree = ast.parse(text)
    except SyntaxError:
        tree = None                      # a shell script, not Python
    if tree is not None:
        reads = _PythonStreamReads()
        reads.visit(tree)
        offenders.extend(f"{label}: {entry} opens a rotated stream" for entry in reads.offenders)
    shell = embedded_shell.emitted_shell(text) if tree is not None else text
    offenders.extend(_shell_offenders(shell, label))
    return offenders


def test_python_and_shell_read_streams_through_the_helper() -> None:
    offenders = []
    # `src` is in scope because the gate is first-party Python that reads
    # logs: `just logs` tailed `.capsem/run/service.log` by name and showed an
    # empty file, and it sat outside this guard's reach the whole time.
    roots = [PROJECT_ROOT / "tests", PROJECT_ROOT / "scripts", PROJECT_ROOT / "src"]
    scanned = 0
    for root in roots:
        for path in sorted(root.rglob("*")):
            if not path.is_file() or path.suffix not in {".py", ".sh"}:
                continue
            if path.name == Path(__file__).name or path.name == "log_streams.py":
                continue
            scanned += 1
            offenders.extend(
                stream_offenders(
                    path.read_text(encoding="utf-8", errors="ignore"),
                    str(path.relative_to(PROJECT_ROOT)),
                )
            )

    assert scanned > 50, "scanned too few files to trust this guard"
    assert not offenders, (
        "read rotated streams through tests/log_streams.py (Python) or by "
        "globbing `<name>*.log` (shell); the bare name is empty after "
        "rotation:\n  " + "\n  ".join(offenders)
    )


# ---------------------------------------------------------------------------
# The guard's own tests, run against the source that shipped the bug it
# exists to prevent. The tree passes today; that says nothing about whether
# the guard would have stopped what got through it.
# ---------------------------------------------------------------------------

#: The exact shape `scripts/local-release-glowup.py` shipped in `b7b9cfbc`,
#: written *while fixing* the macOS instance of this bug. Two commits after
#: the same lesson, in a proof of the same behaviour: three minutes waiting on
#: a file that never existed.
SHIPPED = '''
service_log() {
  echo "$CAPSEM_HOME_DIR/run/service.log"
}

wait_for_rejection() {
  if grep -Fq "automatic release update failed" "$(service_log)" 2>/dev/null; then
    return 0
  fi
  tail -80 "$(service_log)" >&2 2>&1 || echo "no $(service_log)" >&2
}
'''


def test_the_guard_catches_the_shape_that_got_through_it() -> None:
    """A function returning the path is a binding, and `grep` is a read.

    Neither was modelled: `SH_STREAM_BINDING` only matched `NAME=...`, and
    `grep` was absent from `SH_STREAM_READ` while being present in the
    binding-follow list -- one inconsistency, and it is the verb that shipped.
    """
    found = stream_offenders(SHIPPED, "local-release-glowup.py")
    assert found, (
        "the guard still does not see the shape that got past it; widening it "
        "is the fix, not adding a second guard beside it"
    )
    assert len(found) == 2, f"both the grep and the tail must be seen: {found}"
    assert all("indirectly" in entry for entry in found), found


def test_the_fix_for_that_shape_passes() -> None:
    """Globbing the stream is what the guard is asking for."""
    fixed = SHIPPED.replace(
        'echo "$CAPSEM_HOME_DIR/run/service.log"',
        'ls -1t "$CAPSEM_HOME_DIR/run"/service*.log',
    )
    assert stream_offenders(fixed) == []


@pytest.mark.parametrize("verb", ["tail", "cat", "head", "grep", "awk", "sed", "wc"])
def test_every_way_of_reading_a_file_counts_as_a_read(verb: str) -> None:
    """The point is the read, not which tool performs it.

    `grep` was missing for exactly as long as it took someone to reach for it.
    """
    assert stream_offenders(f'{verb} -n "$HOME/.capsem/run/service.log" \n')


def test_the_shipped_macos_shape_is_still_caught() -> None:
    """The variable form this guard was originally written for."""
    source = 'SERVICE_LOG="$CAPSEM_HOME/run/service.log"\ntail -f "$SERVICE_LOG"\n'
    assert stream_offenders(source)


def test_writing_to_the_stream_name_is_not_a_read() -> None:
    """Writers pass the stream name -- that is what the appender expects."""
    assert stream_offenders('capsem-service --log "$HOME/.capsem/run/service.log"\n') == []


# ---------------------------------------------------------------------------
# What parsing buys over the five regexes this replaced. Each case below is a
# way of writing the same read that a pattern had to be told about separately,
# and three of them are ways it was never told.
# ---------------------------------------------------------------------------


def test_a_command_wrapped_for_line_length_is_still_a_command() -> None:
    """Every regex here carried `[^\\n|]*?`, so a wrapped read was invisible.

    Shell joins continuations before it runs anything; a guard that does not
    is reading a different program from the one that executes.
    """
    assert stream_offenders(
        'grep -Fq "needle" \\\n  "$HOME/.capsem/run/service.log"\n'
    )


def test_a_shell_keyword_does_not_hide_the_verb() -> None:
    """`if grep ...` is a grep. A verb-position check that does not know the
    keywords sees `if` and moves on -- and every wait in the glow-up is
    written `if grep ...; then`."""
    for prefix in ("if", "while", "until", "!"):
        assert stream_offenders(f'{prefix} grep -q x "$HOME/.capsem/run/service.log"\n'), prefix


def test_quoting_and_spacing_stop_mattering() -> None:
    """One command, four spellings, one verdict."""
    spellings = [
        'tail -f "$HOME/.capsem/run/service.log"\n',
        "tail -f '$HOME/.capsem/run/service.log'\n",
        "tail    -f     $HOME/.capsem/run/service.log\n",
        'tail -f "${HOME}/.capsem/run/service.log"\n',
    ]
    assert all(stream_offenders(text) for text in spellings)


def test_a_comment_naming_the_stream_is_not_a_read() -> None:
    """Explaining the bug must not trip the guard against it.

    Half the lines in this repository that mention `service.log` are comments
    saying why you must not read it that way.
    """
    assert stream_offenders('# tail "$HOME/.capsem/run/service.log" is wrong\n') == []


def test_a_function_body_containing_a_brace_is_read_whole() -> None:
    """`${VAR}` inside a body ended the regex that matched function bodies.

    It would have read `f() {{ echo "${{HOME}}/x.log"; }}` as ending after
    `${{HOME`, and missed everything after it.
    """
    text = (
        'service_log() {\n  echo "${HOME}/.capsem/run/service.log"\n}\n'
        'grep -q needle "$(service_log)"\n'
    )
    assert stream_offenders(text)


def test_a_stream_a_module_wrote_itself_is_a_fixture() -> None:
    """A test that authored the file knows what is in it.

    Not an exemption by filename: the write and the read are different paths
    in the case that motivated this -- one writes `run_dir/vm/serial.log` and
    reads back the copy under `preserved/`.
    """
    text = (
        "(workspace.run_dir / 'vm' / 'serial.log').write_text('panic')\n"
        "assert (workspace.preserved / 'vm' / 'serial.log').read_text() == 'panic'\n"
    )
    assert stream_offenders(text) == []


def test_reading_a_stream_nothing_wrote_is_still_caught() -> None:
    """The other half of the fixture rule, or it becomes a way out."""
    assert stream_offenders("(run_dir / 'service.log').read_text()\n")


def test_a_binding_two_hundred_lines_from_its_read_is_followed() -> None:
    """`self._log_path` was set in a constructor and read in a method."""
    text = (
        "class Gateway:\n"
        "    def __init__(self, run):\n"
        "        self._log_path = run / 'gateway.log'\n"
        "\n"
        "    def logs(self):\n"
        "        return self._log_path.read_text()\n"
    )
    assert stream_offenders(text)
