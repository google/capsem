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
SH_STREAM_READ = re.compile(r"""(?:tail|cat|head)\s+[^\n|]*?/(?:service|gateway|mcp|tray)\.log["']?\s""")

# Shell hides the same read behind a variable. `SERVICE_LOG=".../service.log"`
# was tailed for three minutes in the macOS glow-up while the line it waited for
# sat in `service.<date>.log`, so a working tamper rejection was reported as a
# failure to reject.
SH_STREAM_BINDING = re.compile(
    r"""^\s*(\w+)=["']?[^\n]*?/(?:service|gateway|mcp|tray)\.log["']?\s*$""",
    re.M,
)


def test_python_and_shell_read_streams_through_the_helper() -> None:
    offenders = []
    roots = [PROJECT_ROOT / "tests", PROJECT_ROOT / "scripts"]
    scanned = 0
    for root in roots:
        for path in sorted(root.rglob("*")):
            if not path.is_file() or path.suffix not in {".py", ".sh"}:
                continue
            if path.name == Path(__file__).name or path.name == "log_streams.py":
                continue
            scanned += 1
            text = path.read_text(encoding="utf-8", errors="ignore")
            for pattern, how in ((PY_STREAM_READ, "read_text"), (SH_STREAM_READ, "shell")):
                for match in pattern.finditer(text):
                    line = text[: match.start()].count("\n") + 1
                    offenders.append(
                        f"{path.relative_to(PROJECT_ROOT)}:{line} reads a rotated "
                        f"stream by name ({how})"
                    )
            for binding in SH_STREAM_BINDING.finditer(text):
                name = binding.group(1)
                for read in re.finditer(
                    rf"""(?:tail|cat|head|grep)\b[^\n|]*?["']?\$\{{?{re.escape(name)}\b""",
                    text,
                ):
                    line = text[: read.start()].count("\n") + 1
                    offenders.append(
                        f"{path.relative_to(PROJECT_ROOT)}:{line} reads log path "
                        f"`${name}` directly (shell)"
                    )
            for binding in PY_STREAM_BINDING.finditer(text):
                name = binding.group(1)
                for read in re.finditer(
                    rf"{re.escape(name)}\.read_text\b", text
                ):
                    line = text[: read.start()].count("\n") + 1
                    offenders.append(
                        f"{path.relative_to(PROJECT_ROOT)}:{line} reads log path "
                        f"`{name}` directly"
                    )

    assert scanned > 50, "scanned too few files to trust this guard"
    assert not offenders, (
        "read rotated streams through tests/log_streams.py (Python) or by "
        "globbing `<name>*.log` (shell); the bare name is empty after "
        "rotation:\n  " + "\n  ".join(offenders)
    )
