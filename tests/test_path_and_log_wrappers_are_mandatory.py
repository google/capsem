"""Capsem paths and log streams are reached through their wrapper, never by hand.

Two rules that exist because both were broken the same way -- a rule living in
a caller instead of a function:

**Environment.** `CAPSEM_RUN_DIR` and `CAPSEM_ASSETS_DIR` each take precedence
over the value derived from `CAPSEM_HOME`. A fixture that sets the home alone
leaves production code reading whichever directories the caller exported, and
`just test` exports them. That passes in a bare shell and fails only inside the
gate. `paths::CapsemPathsGuard::redirect(root)` sets all three together, so
setting one and forgetting the others is not expressible.

**Logs.** `<run>/service.log` names a daily-rotated stream, not a file. Opening
it directly returns nothing, which is how `/service-logs` came to report an
empty log for a service that was writing normally.
`telemetry::read_log_tail` resolves the stream and tails it.

A wrapper nobody is obliged to use is a suggestion. These make it the only way.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

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


# Python and shell read the same rotated streams as Rust, and were not covered
# until an ironbank ledger test asserted on an empty `service.log`.
PY_STREAM_READ = re.compile(
    r"""\(\s*[\w.]+\s*/\s*["'](?:service|gateway|mcp|tray)\.log["']\s*\)\s*\.read_text"""
)

# The same read, one variable removed. `self._log_path` was bound to the
# gateway stream in the constructor and read two hundred lines away, so the
# single-expression pattern above never saw it -- the helper returned "" for a
# gateway that had logged normally, and an ironbank test asserted against the
# empty string. The Rust half has tracked bindings from the start; this is the
# Python equivalent.
PY_STREAM_BINDING = re.compile(
    r"""(\w+(?:\.\w+)*)\s*=\s*[^\n=]*?/\s*["'](?:service|gateway|mcp|tray)\.log["']"""
)
#: `grep` was missing from this list while being present in the
#: binding-follow list below -- one inconsistency, and it is the verb the
#: Linux glow-up used. Every way of reading a file belongs here, because the
#: point is the read, not which tool performs it.
SH_STREAM_VERBS = r"tail|cat|head|grep|awk|sed|less|wc"
SH_STREAM_READ = re.compile(
    rf"""(?:{SH_STREAM_VERBS})\s+[^\n|]*?/(?:service|gateway|mcp|tray)\.log["']?\s"""
)

# Shell hides the same read behind a variable. `SERVICE_LOG=".../service.log"`
# was tailed for three minutes in the macOS glow-up while the line it waited for
# sat in `service.<date>.log`, so a working tamper rejection was reported as a
# failure to reject.
SH_STREAM_BINDING = re.compile(
    r"""^\s*(\w+)=["']?[^\n]*?/(?:service|gateway|mcp|tray)\.log["']?\s*$""",
    re.M,
)

# ...and behind a function, which is the shape that got through. The Linux
# glow-up wrote
#
#     service_log() { echo "$CAPSEM_HOME_DIR/run/service.log"; }
#     grep -Fq "..." "$(service_log)"
#
# to fix the macOS bug this guard was written for, and reproduced it exactly:
# not a variable, so `SH_STREAM_BINDING` saw nothing, and `grep`, which
# `SH_STREAM_READ` did not list. Three minutes waiting on a file that never
# existed, in a proof of the same behaviour, two commits after the same lesson.
#
# A function returning the path is a binding. Modelling it here rather than
# adding a second guard beside this one is the whole point: the hole was in the
# mechanism, not in the coverage.
SH_STREAM_FUNCTION = re.compile(
    r"""^\s*(\w+)\s*\(\)\s*\{[^}]*?/(?:service|gateway|mcp|tray)\.log""",
    re.M | re.S,
)


def stream_offenders(text: str, label: str = "<text>") -> list[str]:
    """Every rotated-stream read in one file, however it is spelled.

    Extracted so the guard can be run against source that is not in the tree
    -- above all the version that shipped the bug it exists to prevent. A
    guard only exercised on a tree that already passes is a guard nobody has
    watched fail.
    """
    offenders: list[str] = []
    for pattern, how in ((PY_STREAM_READ, "read_text"), (SH_STREAM_READ, "shell")):
        for match in pattern.finditer(text):
            line = text[: match.start()].count("\n") + 1
            offenders.append(
                f"{label}:{line} reads a rotated "
                f"stream by name ({how})"
            )
    for binding in SH_STREAM_BINDING.finditer(text):
        name = binding.group(1)
        for read in re.finditer(
            rf"""(?:{SH_STREAM_VERBS})\b[^\n|]*?["']?\$\{{?{re.escape(name)}\b""",
            text,
        ):
            line = text[: read.start()].count("\n") + 1
            offenders.append(
                f"{label}:{line} reads log path "
                f"`${name}` directly (shell)"
            )
    for function in SH_STREAM_FUNCTION.finditer(text):
        name = function.group(1)
        for read in re.finditer(
            rf"""(?:{SH_STREAM_VERBS})\b[^\n|]*?\$\(\s*{re.escape(name)}\s*\)""",
            text,
        ):
            line = text[: read.start()].count("\n") + 1
            offenders.append(
                f"{label}:{line} reads log path "
                f"`$({name})` directly (shell function)"
            )
    for binding in PY_STREAM_BINDING.finditer(text):
        name = binding.group(1)
        for read in re.finditer(
            rf"{re.escape(name)}\.read_text\b", text
        ):
            line = text[: read.start()].count("\n") + 1
            offenders.append(
                f"{label}:{line} reads log path "
                f"`{name}` directly"
            )
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
    assert any("shell function" in entry for entry in found), found


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
