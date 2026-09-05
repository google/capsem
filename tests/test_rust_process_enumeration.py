"""Rust reads the process table with a syscall, never by spawning `ps`.

`/bin/ps` is setuid root (`-rwsr-xr-x root wheel`), and macOS refuses to
`execvp` a setuid binary from a sandboxed process no matter what the profile
permits -- even `(allow default)` gives `Operation not permitted`. So any code
that shells out to `ps` silently stops working the moment it runs under the
release gate's Seatbelt profile.

Silently is the whole problem. `reap_orphan_capsem_processes` matched on
`Ok(o) if o.status.success() => o, _ => return`, so a failed spawn was
indistinguishable from "no orphans found". A service restarting after a crash
reaped none of the per-VM `capsem-process` children it exists to reap; they
outlived the gate by half an hour, still holding their run directories, and
`orphan-accounting` failed the run at teardown with no hint of the cause.

This is the third copy of one defect. `capsem_builder.gate.pidfiles` shelled out to
`ps` and was moved to `proc_pidinfo`; the tray-singleton test carried its own
copy; this was the third. Duplicated knowledge diverges, and the copy is
always the one nobody rechecks -- so the invariant is a test rather than three
fixed call sites.

The syscall-backed `capsem_foundation::proctable` is the replacement. It uses
`libproc` plus `KERN_PROCARGS2` on macOS and `/proc` on Linux, and returns one
typed process snapshot shared by the service and benchmark harness.
"""

from __future__ import annotations

import re
import tomllib
from pathlib import Path

import rust_sources

PROJECT_ROOT = Path(__file__).resolve().parents[1]

#: `Command::new("ps")`, `Command::new("/bin/ps")`, and the `std::process::`
#: spelling of either. Matching the constructor rather than the bare word
#: keeps `ps` in prose, in a variable name, or in a guest-side script from
#: tripping it.
_SPAWNS_PS = re.compile(r"""Command::new\(\s*"(?:/bin/|/usr/bin/)?ps"\s*\)""")


def _rust_production_sources() -> list[Path]:
    return sorted(
        path
        for path in (PROJECT_ROOT / "crates").rglob("*.rs")
        if path.name != "tests.rs" and "/cache/target/" not in str(path)
    )


def test_no_rust_source_shells_out_to_ps() -> None:
    """A sandboxed process cannot exec setuid `ps`, so nothing may depend on it.

    The failure this prevents is not a crash. It is a best-effort helper that
    returns "nothing found" forever, under exactly the conditions -- a release
    gate -- where its answer is load-bearing.
    """
    offenders = [
        f"{path.relative_to(PROJECT_ROOT)}:{index}"
        for path in _rust_production_sources()
        for index, line in enumerate(rust_sources.production(path).splitlines(), start=1)
        if _SPAWNS_PS.search(line)
    ]
    assert not offenders, (
        "these spawn setuid `ps`, which macOS refuses to exec from a sandboxed "
        "process, so they silently find nothing under the release gate: "
        f"{offenders}. Read the shared process table instead -- see "
        "`capsem_foundation::proctable`."
    )


def test_guest_benchmark_uses_only_the_foundation_process_table() -> None:
    """One process-table reader must not pull the host runtime into the guest."""
    foundation = tomllib.loads(
        (PROJECT_ROOT / "crates/capsem-foundation/Cargo.toml").read_text()
    )
    benchmark = tomllib.loads(
        (PROJECT_ROOT / "crates/capsem-bench/Cargo.toml").read_text()
    )

    dependency = benchmark["dependencies"]["capsem-foundation"]
    assert dependency["default-features"] is False
    assert dependency["optional"] is True
    assert benchmark["dependencies"]["rusqlite"]["optional"] is True
    assert benchmark["features"]["guest"] == []
    assert foundation["features"]["default"] == ["runtime"]
