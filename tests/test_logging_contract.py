"""Contracts that keep a dead daemon explainable.

Tracing only records what a live process chose to say. The failures that
matter most -- a panic, a startup that dies before `telemetry::init` returns
-- reach the process's stderr and nothing else, so where that stderr goes
decides whether the failure is debuggable at all.
"""

from __future__ import annotations

import re
from pathlib import Path

from rust_sources import production


PROJECT_ROOT = Path(__file__).resolve().parent.parent
SERVICE_MAIN = PROJECT_ROOT / "crates" / "capsem-service" / "src" / "main.rs"
GATEWAY_MAIN = PROJECT_ROOT / "crates" / "capsem-gateway" / "src" / "main.rs"
CLI_CLIENT = PROJECT_ROOT / "crates" / "capsem" / "src" / "client.rs"
TELEMETRY = PROJECT_ROOT / "crates" / "capsem-core" / "src" / "telemetry.rs"

# Run-dir subdirectory holding raw process stderr, kept out of the rotated
# streams' directory so log retention cannot delete it.
STDERR_SUBDIR = "stderr"


def test_long_lived_daemons_route_panics_into_their_log() -> None:
    """A panicking daemon must say so somewhere a human will look.

    Without a hook the message goes to stderr, which for the service is a
    file it does not own and, on the direct-spawn path, was /dev/null.
    """
    assert "pub fn install_panic_logger(" in production(TELEMETRY)

    for daemon in (SERVICE_MAIN, GATEWAY_MAIN):
        source = production(daemon)
        assert "install_panic_logger(" in source, (
            f"{daemon.name} does not install the panic logger"
        )
        # One definition, not a re-implementation per binary.
        assert "std::panic::set_hook" not in source, (
            f"{daemon.name} hand-rolls a panic hook instead of using the shared one"
        )


def test_direct_spawned_service_keeps_its_stderr() -> None:
    """The CLI's direct spawn must not discard the service's stderr.

    Detaching stdio is required -- an inherited pipe makes every
    `capsem run` under a capturing harness hang until timeout -- but
    detaching to /dev/null throws away panics and pre-tracing startup
    failures, the two cases where service.log is empty by definition.
    """
    source = production(CLI_CLIENT)

    spawn = source[source.index("spawning service directly") :]
    spawn = spawn[: spawn.index("failed to spawn capsem-service")]

    assert ".stdin(std::process::Stdio::null())" in spawn, (
        "stdin must stay detached"
    )
    assert ".stderr(std::process::Stdio::null())" not in spawn, (
        "service stderr is discarded; a panic leaves no record anywhere"
    )
    assert "service.log" in spawn, (
        "service stderr should land in a file, not be thrown away"
    )
    # Rotation prunes by `starts_with(prefix) && ends_with(suffix)`, so a raw
    # stderr file sitting beside the rotated stream is itself a prune
    # candidate. Unlinking it would not disturb the already-open fd -- the
    # service would keep writing panics into an inode nobody can open.
    assert f'join("{STDERR_SUBDIR}")' in spawn, (
        f"raw stderr must live under {STDERR_SUBDIR}/, out of reach of the "
        "rotation pruner that owns the run dir"
    )


def _rust_sources() -> list[Path]:
    return [
        path
        for path in (PROJECT_ROOT / "crates").rglob("*.rs")
        if "target" not in path.parts
    ]


# A tracing macro whose message text interpolates the error value itself.
INTERPOLATED_ERROR = re.compile(
    r"""(?:tracing::)?(?:error|warn|info|debug|trace)!\(   # a tracing macro
        [^)"]*                                            # any leading fields
        "[^"]*\{(?:e|err|error)(?::[^}]*)?\}               # {e} / {err} / {error:#}
    """,
    re.VERBOSE,
)

# `error = format!("{e:#}")` is the sanctioned way to keep an anyhow cause
# chain in a field -- tracing has no alternate-Display sigil. It contains the
# same `{e:#}` the rule forbids in a message, so lift it out before matching:
# a guard that flags the fix it recommends teaches people to disable it.
ERROR_CHAIN_FIELD = re.compile(r"""format!\("[^"]*"\)""")


def test_errors_are_logged_as_fields_not_baked_into_the_message() -> None:
    """An error belongs in `error = %e`, never inside the message string.

    JSON-per-line output is only structured if the parts you filter on are
    fields. An error interpolated into the message cannot be grouped, counted,
    or alerted on -- every distinct path or errno makes a distinct message --
    and it is the single field a reader of a bug report looks for first.
    """
    offenders: list[str] = []
    for path in _rust_sources():
        for number, line in enumerate(
            path.read_text(encoding="utf-8").splitlines(), start=1
        ):
            if INTERPOLATED_ERROR.search(ERROR_CHAIN_FIELD.sub("", line)):
                rel = path.relative_to(PROJECT_ROOT)
                offenders.append(f"{rel}:{number}: {line.strip()}")

    assert not offenders, (
        f"{len(offenders)} log call(s) bake the error into the message; "
        "use `error = %e` (or `error = format!(\"{e:#}\")` to keep an anyhow "
        "cause chain) so it stays a field:\n" + "\n".join(offenders[:20])
    )
