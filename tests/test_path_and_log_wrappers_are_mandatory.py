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
    r"""(?:std::env::)?set_var\(\s*["'](%s)["']""" % "|".join(MANAGED_VARS)
)

# Owns the rule, so it is the one place allowed to implement it.
ENV_OWNER = "paths.rs"
LOG_OWNER = "telemetry.rs"

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

    # `let <name> = ... .join("<something>.log")`
    binding = re.compile(
        r"""let\s+(?:mut\s+)?(\w+)\s*(?::[^=]+)?=\s*[^;]*?\.join\(\s*"""
        r"""(?:&?format!\()?["'][a-z_]+\.log["'][^;]*;""",
        re.S,
    )

    offenders = []
    for path in sources:
        if path.name == LOG_OWNER:
            continue
        text = path.read_text(encoding="utf-8", errors="ignore")
        if "read_log_tail" in text or "log_stream_files" in text:
            continue
        # Per function body: a binding named `path` in one function says
        # nothing about a parameter named `path` in another.
        for body in re.split(r"\n(?=\s*(?:pub\s+)?(?:async\s+)?fn\s)", text):
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
