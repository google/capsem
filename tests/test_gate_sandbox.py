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
from helpers.gate import RecordingRunner
from pydantic import ValidationError

from capsem.gate import config as gate_config
from capsem.gate import egress, sandbox
from capsem.gate.actions import Run
from capsem.gate.context import Context
from capsem.gate.errors import GateError
from capsem.gate.harnessschema import SandboxConfig

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)

macos_only = pytest.mark.skipif(sys.platform != "darwin", reason="Seatbelt is macOS")
linux_only = pytest.mark.skipif(sys.platform != "linux", reason="Bubblewrap is Linux")

#: A sandbox cannot be nested: `sandbox-exec: sandbox_apply: Operation not
#: permitted` once already inside one. So the two tests that really invoke it
#: cannot run inside a sandboxed gate -- which is where they first ran, and
#: where they first failed. Detected by asking the kernel rather than by a
#: marker the gate would have to remember to export.
def _already_sandboxed() -> bool:
    if sys.platform != "darwin":
        return False
    probe = subprocess.run(
        ["sandbox-exec", "-p", "(version 1)(allow default)", "true"],
        capture_output=True,
        text=True,
    )
    return "sandbox_apply" in probe.stderr


unnested_only = pytest.mark.skipif(
    _already_sandboxed(), reason="a sandbox cannot be applied inside a sandbox"
)


def test_the_profile_denies_the_network() -> None:
    """The rule the whole profile exists for."""
    text = sandbox.profile(CONFIG, report=False)

    assert "(allow default)" in text
    assert "(deny network*)" in text


def test_report_mode_permits_and_logs_rather_than_denying() -> None:
    """`(with report)` is a modifier on *allow*. It is not one on deny.

    `(deny network* (with report))` is refused outright -- `sandbox-exec:
    report modifier does not apply to deny action` -- and the run dies before
    it starts. This cost a gate launch to learn, which is cheap only because
    the failure is immediate.

    So measuring is "permit everything and log it", not "deny and log". That
    is also what makes one run enough: nothing is refused, so nothing stops
    early, and what comes back is the complete surface rather than the first
    thing that happened to be reached.
    """
    reporting = sandbox.profile(CONFIG, report=True)

    assert "(allow network* (with report))" in reporting
    assert "(deny network*)" not in reporting, "report mode must refuse nothing"

    # And it is the last rule, because a later one wins in SBPL: the specific
    # socket allows would silence reporting for exactly the paths already
    # known, and the ones worth learning about are the rest.
    assert reporting.strip().endswith("(allow network* (with report))")


def test_enforcing_mode_denies_and_names_what_comes_back() -> None:
    """The other half, and the shape that actually runs a gate."""
    enforcing = sandbox.profile(CONFIG, report=False)

    assert "(deny network*)" in enforcing
    assert "(with report)" not in enforcing
    # After the denial, so they win: SBPL takes the last matching rule.
    assert enforcing.index("(deny network*)") < enforcing.index("(allow network* (literal")


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
@unnested_only
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
@unnested_only
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


def test_linux_enforcement_uses_bubblewrap_network_namespace(monkeypatch) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("shutil.which", lambda _name: "/usr/bin/bwrap")
    monkeypatch.setattr("capsem.gate.sandbox.active", lambda _config: False)

    wrapped = sandbox.applied(
        CONFIG,
        RecordingRunner(PROJECT_ROOT),
        default=sandbox.ENFORCE,
        requested=None,
        argv=("python3", "-c", "print('inside')"),
    )

    assert wrapped[0] == CONFIG.sandbox.linux_command
    assert tuple(CONFIG.sandbox.linux_args) == wrapped[1 : 1 + len(CONFIG.sandbox.linux_args)]
    assert "--unshare-net" in wrapped
    assert wrapped[
        wrapped.index("--bind") : wrapped.index("--bind") + 3
    ] == ("--bind", "/", "/")
    assert wrapped[-3:] == ("python3", "-c", "print('inside')")


@pytest.mark.parametrize(
    "linux_args",
    [
        ("--die-with-parent", "--new-session", "--bind", "/", "/"),
        ("--unshare-net", "--new-session", "--bind", "/", "/"),
        ("--unshare-net", "--die-with-parent", "--new-session"),
    ],
)
def test_linux_sandbox_config_cannot_remove_an_enforcement_property(
    linux_args: tuple[str, ...],
) -> None:
    raw = CONFIG.sandbox.model_dump()
    raw["linux_args"] = linux_args

    with pytest.raises(ValidationError):
        SandboxConfig.model_validate(raw)


def test_linux_report_mode_refuses_to_claim_unimplemented_observation(monkeypatch) -> None:
    monkeypatch.setattr("capsem.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem.gate.sandbox.active", lambda _config: False)

    with pytest.raises(GateError, match="report mode"):
        sandbox.applied(
            CONFIG,
            RecordingRunner(PROJECT_ROOT),
            default=sandbox.REPORT,
            requested=None,
            argv=("true",),
        )


@linux_only
def test_linux_kernel_boundary_keeps_loopback_and_denies_external_interfaces() -> None:
    probe = """
import socket
import threading

interfaces = {name for _index, name in socket.if_nameindex()}
assert interfaces == {"lo"}, interfaces

server = socket.socket()
server.bind(("127.0.0.1", 0))
server.listen(1)
port = server.getsockname()[1]
thread = threading.Thread(
    target=lambda: socket.create_connection(("127.0.0.1", port), timeout=1).close()
)
thread.start()
connection, _address = server.accept()
connection.close()
server.close()
thread.join(timeout=1)
assert not thread.is_alive()

try:
    socket.create_connection(("1.1.1.1", 443), timeout=0.2)
except OSError:
    pass
else:
    raise AssertionError("external AF_INET connection escaped the Linux gate boundary")
"""
    argv = sandbox.applied(
        CONFIG,
        RecordingRunner(PROJECT_ROOT),
        default=sandbox.ENFORCE,
        requested=None,
        argv=(sys.executable, "-c", probe),
    )

    completed = subprocess.run(argv, capture_output=True, text=True, timeout=10)

    assert completed.returncode == 0, completed.stderr


@linux_only
def test_docker_answers_inside_the_linux_kernel_boundary() -> None:
    argv = sandbox.applied(
        CONFIG,
        RecordingRunner(PROJECT_ROOT),
        default=sandbox.ENFORCE,
        requested=None,
        argv=("docker", "version", "--format", "{{.Server.Version}}"),
    )

    answered = subprocess.run(argv, capture_output=True, text=True, timeout=30)

    assert answered.returncode == 0, answered.stderr
    assert answered.stdout.strip(), "Docker daemon answered with no version"


def test_the_sandbox_is_applied_before_any_resource_is_held() -> None:
    """A profile is inherited and cannot be dropped, so where it is applied is
    the whole design.

    Applied in-process it would sandbox the parent that still has to reclaim
    the private copy and write the summary. Applied *after* the machine lock,
    the sandboxed child asks for the lock its own parent holds and waits out
    the full 7200-second timeout -- the same deadlock the private copy and the
    keep-awake wrapper are placed to avoid, which is why all three live at the
    one seam `execute` calls before `held`.

    Asserted on the source order rather than by running a gate, because the
    failure is a two-hour hang: a test that reproduced it would take longer
    than the run it protects.
    """
    import ast

    source = (PROJECT_ROOT / "src" / "capsem" / "gate" / "command.py").read_text(encoding="utf-8")
    tree = ast.parse(source)
    execute = next(
        node
        for node in ast.walk(tree)
        if isinstance(node, ast.FunctionDef) and node.name == "execute"
    )

    def line_of(fragment: str) -> int:
        # The *first* line, by source position. `ast.walk` is breadth-first,
        # so taking whichever node it yields first compares arbitrary lines --
        # which is how the first version of this guard passed a mutation that
        # moved the re-exec inside the block it is supposed to precede.
        found = [
            node.lineno
            for node in ast.walk(execute)
            if isinstance(node, ast.Call) and fragment in ast.unparse(node)
        ]
        assert found, f"{fragment!r} is not called in execute()"
        return min(found)

    reexec_at = line_of("self.reexec")
    held_at = line_of("held")
    assert reexec_at < held_at, (
        "the re-exec that applies the sandbox happens after resources are "
        "held, so the sandboxed child waits out its own parent's machine lock"
    )

    # And outside the `with` entirely, not merely on an earlier line than it.
    # A re-exec nested inside the recording or holding block still leaves the
    # parent holding what the child is about to ask for.
    for node in ast.walk(execute):
        if not isinstance(node, ast.With):
            continue
        nested = [
            inner.lineno
            for inner in ast.walk(node)
            if isinstance(inner, ast.Call) and "self.reexec" in ast.unparse(inner)
        ]
        assert not nested, (
            f"the re-exec is inside a `with` block opened at line {node.lineno}; "
            "it must run before anything is acquired at all"
        )


def test_off_is_the_default_so_a_short_command_keeps_its_network() -> None:
    """The profile denies the network, and most commands are short reads.

    Declared per command rather than globally, because turning it on for
    everything means rediscovering which socket each one needed -- at the cost
    of a run each time.
    """
    from helpers.gate import built_command

    from capsem.gate.command import GateCommand

    # Through the helper, which is the one place that knows importing `cli` is
    # what fills the registry -- spelling that import here needs a suppression
    # for a name nothing reads.
    assert built_command(PROJECT_ROOT, "runs", (("limit", None),)).sandboxed == sandbox.OFF
    assert GateCommand.sandboxed == sandbox.OFF
    assert sandbox.mode(sandbox.OFF, None) == sandbox.OFF
    # And an explicit request wins over the declaration, which is what makes
    # report mode a measurement rather than an edit.
    assert sandbox.mode(sandbox.OFF, sandbox.REPORT) == sandbox.REPORT


def test_an_outside_action_uses_only_the_capability_runner() -> None:
    ordinary = RecordingRunner(PROJECT_ROOT)
    capability = RecordingRunner(PROJECT_ROOT)

    Run(("python3", "-c", "print('networked')"), outside_sandbox=True).perform(
        Context(ordinary, CONFIG, outside_runner=capability)
    )

    assert ordinary.commands == []
    assert capability.rendered == ["python3 -c 'print('\"'\"'networked'\"'\"')'"]


@linux_only
def test_the_egress_broker_crosses_the_boundary_without_giving_children_its_capability(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The plan executor keeps one narrow capability; its children do not.

    Host interface enumeration is deterministic and needs no public network:
    the ordinary command sees only the Bubblewrap namespace while the marked
    command is executed by the helper that existed before the boundary.
    """
    metadata = egress.prepare(CONFIG, tmp_path)
    monkeypatch.setenv(CONFIG.sandbox.egress_metadata_variable, str(metadata))
    resource = egress.Egress(CONFIG, enabled=True)
    resource.acquire()
    try:
        probe = "import socket; print(','.join(n for _, n in socket.if_nameindex()))"
        inside = subprocess.run(
            sandbox.linux_wrap(CONFIG, (sys.executable, "-c", probe)),
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
        outside = resource.runner.capture((sys.executable, "-c", probe))

        assert inside == "lo"
        assert set(outside.split(",")) != {"lo"}
        assert not metadata.exists(), "the child-readable capability survived acquisition"
    finally:
        resource.release()
