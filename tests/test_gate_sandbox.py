"""The profile a run executes under, and the two facts that make it work.

Every other isolation here is enforced by the gate's own code, which means a
mistake in that code can undo it. This one is enforced by the kernel: it is
what turns "the gate fetched nothing mid-run" from a claim into a property.

The shape is `(allow default)` narrowed by targeted denials, and that is a
measured decision rather than a preference. `(deny default)` widened by
enumerated allows was tried: every read list produced a silent `SIGABRT` --
exit 134, no output at all -- and the kernel's denial log needs sudo, so each
attempt cost a rebuild to learn nothing.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import pytest

from capsem.gate import config as gate_config
from capsem.gate import sandbox

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)

macos_only = pytest.mark.skipif(sys.platform != "darwin", reason="Seatbelt is macOS")


def test_the_profile_denies_the_network() -> None:
    """The rule the whole profile exists for."""
    text = sandbox.profile(CONFIG, report=False)

    assert "(allow default)" in text
    assert "(deny network*)" in text


def test_report_mode_differs_only_by_reporting() -> None:
    """8a measures and 8b enforces, and they must be the same profile.

    A report run whose rules differ from the enforcing one measures a profile
    nobody will run -- which is the failure mode of collecting an allow-list
    at all: forty iterations that each discover one more thing, against one
    run that discovers all of them.
    """
    reporting = sandbox.profile(CONFIG, report=True)
    enforcing = sandbox.profile(CONFIG, report=False)

    assert "(deny network* (with report))" in reporting
    assert reporting.replace(" (with report)", "") == enforcing


def test_every_rule_comes_from_configuration() -> None:
    """A literal path in a security profile is a rule nobody can find.

    `test_gate_has_no_literal_data` enforces this for the module already; this
    asserts the other direction -- that what is configured actually reaches the
    profile, so a socket added to `[sandbox]` cannot be silently ignored.
    """
    text = sandbox.profile(CONFIG, report=False)
    settings = CONFIG.sandbox

    assert settings.sockets, "no UNIX socket is allowed back, so Docker will look absent"
    for socket in settings.sockets:
        assert str(Path(socket).expanduser()) in text
    for prefix in settings.local_socket_prefixes:
        assert prefix in text
    for address in settings.loopback:
        assert address in text


def test_the_loopback_rule_names_a_host_sbpl_accepts() -> None:
    """SBPL refuses any host but `*` or `localhost` in a network address.

    `127.0.0.1:*` reads like a narrower rule and is in fact a profile that
    will not load at all: `sandbox-exec: host must be * or localhost in
    network address`, and the run dies before it starts rather than running
    unsandboxed. Found in one second by the real invocation below, which is
    why that test exists.
    """
    for address in CONFIG.sandbox.loopback:
        host = address.split(":", 1)[0]
        assert host in {"*", "localhost"}, (
            f"{address!r} names {host!r}; SBPL accepts only `*` or `localhost`, "
            "so this profile would fail to load"
        )


@macos_only
def test_the_profile_loads_and_denies_what_it_says(tmp_path: Path) -> None:
    """Cheap, real, and the only thing that proves any of the above.

    One second against a kernel, rather than an hour of gate to discover that
    `(deny network*)` also covers the Docker socket -- which it does, and which
    makes a running daemon look absent to whoever is reading the failure.
    """
    written = tmp_path / CONFIG.sandbox.profile_name
    written.write_text(sandbox.profile(CONFIG, report=False), encoding="utf-8")

    def under(*argv: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            sandbox.wrap(CONFIG, written, argv),
            capture_output=True,
            text=True,
            timeout=60,
        )

    # It loads at all. A malformed profile exits 65 before running anything,
    # which is indistinguishable from the command failing if nothing checks.
    loaded = under("true")
    assert loaded.returncode == 0, f"the profile does not load: {loaded.stderr.strip()}"

    denied = under("curl", "-sS", "-m", "5", "-o", "/dev/null", "https://1.1.1.1")
    assert denied.returncode != 0, "the internet is still reachable under the profile"


@macos_only
def test_docker_still_answers_through_its_unix_socket(tmp_path: Path) -> None:
    """The fact that costs a day if it is learned the hard way.

    `(deny network*)` denies AF_UNIX too. Without the explicit allow the first
    sandboxed run reports `permission denied ... docker.sock`, which every
    reader diagnoses as "Docker isn't running" rather than as the sandbox.
    """
    if subprocess.run(["docker", "version"], capture_output=True).returncode != 0:
        pytest.skip("no Docker daemon to ask")

    written = tmp_path / CONFIG.sandbox.profile_name
    written.write_text(sandbox.profile(CONFIG, report=False), encoding="utf-8")

    answered = subprocess.run(
        sandbox.wrap(CONFIG, written, ("docker", "version", "--format", "{{.Server.Version}}")),
        capture_output=True,
        text=True,
        timeout=60,
    )

    assert answered.returncode == 0, (
        "Docker is unreachable under the profile, which is what an unallowed "
        f"UNIX socket looks like: {answered.stderr.strip()}"
    )
    assert answered.stdout.strip(), "the daemon answered with nothing"
