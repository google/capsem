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

import importlib.util
import os
import subprocess
import sys
import threading
import time
from pathlib import Path
from types import ModuleType

import pytest
import variables
import yaml
from capsem_builder.gate import cancellation, egress, sandbox
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.actions import Run, Script
from capsem_builder.gate.config import GateConfig
from capsem_builder.gate.context import Context
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.harnessschema import SandboxConfig
from capsem_builder.gate.proc import Runner
from capsem_builder.gate.processgroup import StopPolicy
from helpers.gate import RecordingRunner
from helpers.workflow_contract import assert_unmasked_step
from pydantic import ValidationError

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)

macos_only = pytest.mark.skipif(sys.platform != "darwin", reason="Seatbelt is macOS")
linux_only = pytest.mark.skipif(sys.platform != "linux", reason="Bubblewrap is Linux")


#: A sandbox cannot be meaningfully nested. Seatbelt refuses the second
#: application, while Bubblewrap cannot create the pre-boundary helper from
#: inside the inherited network namespace. So tests that really invoke the
#: boundary run only from an unsandboxed parent. Detected by asking the kernel
#: rather than by a marker the gate would have to remember to export.
def _already_sandboxed() -> bool:
    if sys.platform == "linux":
        return sandbox.active(CONFIG)
    if sys.platform != "darwin":
        return False
    probe = subprocess.run(
        ["sandbox-exec", "-p", "(version 1)(allow default)", "true"],
        capture_output=True,
        text=True,
    )
    return "sandbox_apply" in probe.stderr


unnested_only = pytest.mark.skipif(
    _already_sandboxed(), reason="the kernel sandbox cannot be applied from inside itself"
)

ONLINE_FAST = {
    "fast.audit.cargo",
    "fast.audit.pnpm",
    "fast.audit.python-lock",
    "fast.toolchain.ort",
    "fast.toolchain.rust",
}


def _linux_sandbox_preparer() -> ModuleType:
    path = PROJECT_ROOT / "scripts" / "prepare-linux-sandbox.py"
    spec = importlib.util.spec_from_file_location("prepare_linux_sandbox", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def test_hosted_linux_sandbox_repairs_only_the_known_apparmor_restriction() -> None:
    module = _linux_sandbox_preparer()
    calls: list[tuple[str, ...]] = []
    results = iter(
        (
            subprocess.CompletedProcess(
                (), 1, "", "bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted"
            ),
            subprocess.CompletedProcess((), 0, "", ""),
            subprocess.CompletedProcess((), 0, "", ""),
        )
    )

    def run(argv: tuple[str, ...], **_kwargs) -> subprocess.CompletedProcess[str]:
        calls.append(argv)
        return next(results)

    module.prepare(
        PROJECT_ROOT,
        allow_hosted_repair=True,
        environment={"GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "github-hosted"},
        run=run,
        read_restriction=lambda _name: "1",
    )

    assert calls[0] == calls[2]
    assert calls[0][: 1 + len(CONFIG.sandbox.linux_args)] == (
        CONFIG.sandbox.linux_command,
        *CONFIG.sandbox.linux_args,
    )
    assert calls[1] == (
        *CONFIG.sandbox.linux_hosted_repair_command,
        f"{CONFIG.sandbox.linux_hosted_userns_sysctl}="
        f"{CONFIG.sandbox.linux_hosted_userns_repair_value}",
    )


def test_hosted_linux_sandbox_requires_the_complete_probe_after_repair() -> None:
    module = _linux_sandbox_preparer()
    calls: list[tuple[str, ...]] = []
    results = iter(
        (
            subprocess.CompletedProcess(
                (), 1, "", "bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted"
            ),
            subprocess.CompletedProcess((), 0, "", ""),
            subprocess.CompletedProcess((), 1, "", "direct egress remained reachable"),
        )
    )

    def run(argv: tuple[str, ...], **_kwargs) -> subprocess.CompletedProcess[str]:
        calls.append(argv)
        return next(results)

    with pytest.raises(module.PreparationError, match="still fails after"):
        module.prepare(
            PROJECT_ROOT,
            allow_hosted_repair=True,
            environment={"GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "github-hosted"},
            run=run,
            read_restriction=lambda _name: "1",
        )
    assert len(calls) == 3
    assert calls[0] == calls[2]


@pytest.mark.parametrize(
    ("stderr", "environment"),
    (
        ("bwrap: creating new namespace failed", {"GITHUB_ACTIONS": "true"}),
        (
            "bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted",
            {"GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "self-hosted"},
        ),
    ),
)
def test_hosted_linux_sandbox_refuses_unknown_or_unhosted_failures(
    stderr: str, environment: dict[str, str]
) -> None:
    module = _linux_sandbox_preparer()
    calls: list[tuple[str, ...]] = []

    def run(argv: tuple[str, ...], **_kwargs) -> subprocess.CompletedProcess[str]:
        calls.append(argv)
        return subprocess.CompletedProcess(argv, 1, "", stderr)

    with pytest.raises(module.PreparationError):
        module.prepare(
            PROJECT_ROOT,
            allow_hosted_repair=True,
            environment=environment,
            run=run,
            read_restriction=lambda _name: "1",
        )
    assert len(calls) == 1, "an unrecognized host failure triggered a privileged repair"


def test_fast_gate_proves_hosted_linux_sandbox_before_dependency_work() -> None:
    workflow_path = PROJECT_ROOT / ".github" / "workflows" / "fast-gate.yaml"
    workflow = yaml.safe_load(workflow_path.read_text())
    steps = workflow["jobs"]["static"]["steps"]
    names = [step.get("name") for step in steps]
    prepare = next(step for step in steps if step.get("name") == "Prove Linux sandbox boundary")

    assert prepare["run"] == ("python3 scripts/prepare-linux-sandbox.py --repair-hosted-runner")
    assert "continue-on-error" not in prepare
    assert names.index("Prove Linux sandbox boundary") < names.index(
        "Materialize locked qualification dependencies"
    )
    assert names.index("Prove Linux sandbox boundary") < names.index(
        "Run the complete fast gate"
    )
    assert_unmasked_step("fast-gate.yaml", workflow, "static", "Prove Linux sandbox boundary")


def test_every_hosted_linux_job_entering_a_gate_module_proves_the_boundary_first() -> None:
    """A package/profile pairing job is just as sandboxed as the fast gate.

    Installing Bubblewrap does not prove that an Ubuntu runner may create the
    namespace or configure loopback.  GitHub's AppArmor restriction can allow
    the former while denying the latter, so every direct module caller must
    use the same narrow repair-and-probe primitive before fetching Rust
    dependencies or entering the gate.
    """
    callers: set[tuple[str, str]] = set()
    for workflow_path in sorted((PROJECT_ROOT / ".github" / "workflows").glob("*.yaml")):
        workflow = yaml.safe_load(workflow_path.read_text(encoding="utf-8"))
        for job_name, job in workflow.get("jobs", {}).items():
            if "ubuntu" not in str(job.get("runs-on", "")):
                continue
            steps = job.get("steps", [])
            # The public verbs that enter a gate module. This looked for
            # `just _test-`, which no workflow contains since CI stopped
            # calling private recipes -- the inventory went empty and the
            # equality assertion below is the only reason that was noticed.
            module_indexes = [
                index
                for index, step in enumerate(steps)
                if any(
                    f"just {verb}" in str(step.get("run", ""))
                    for verb in (
                        variables.FAST_TEST,
                        variables.QUALIFY_ASSETS,
                        variables.QUALIFY_BINARIES,
                    )
                )
            ]
            if not module_indexes:
                continue
            callers.add((workflow_path.name, job_name))
            prepare_indexes = [
                index
                for index, step in enumerate(steps)
                if step.get("name") == "Prove Linux sandbox boundary"
            ]
            assert len(prepare_indexes) == 1, (
                f"{workflow_path.name}:{job_name} directly enters a gate module "
                "without exactly one hosted Linux sandbox proof"
            )
            prepare_index = prepare_indexes[0]
            prepare = steps[prepare_index]
            assert prepare["run"] == (
                "python3 scripts/prepare-linux-sandbox.py --repair-hosted-runner"
            )
            assert prepare_index < min(module_indexes)
            assert all(
                prepare_index < index
                for index, step in enumerate(steps)
                if "cargo fetch" in str(step.get("run", ""))
            ), f"{workflow_path.name}:{job_name} fetched Cargo inputs before sandbox proof"
            assert_unmasked_step(
                workflow_path.name, workflow, job_name, "Prove Linux sandbox boundary"
            )

    assert callers == {
        ("fast-gate.yaml", "static"),
        ("release-assets.yaml", "test-profile-pairing"),
        ("release.yaml", "test-binary-pairing"),
    }


@pytest.mark.parametrize("job_name", ("nightly-release",))
def test_nightly_release_bootstraps_host_before_enforced_qualification(job_name: str) -> None:
    workflow_path = PROJECT_ROOT / ".github" / "workflows" / "release-nightly.yaml"
    workflow = yaml.safe_load(workflow_path.read_text())
    steps = workflow["jobs"][job_name]["steps"]
    names = [step.get("name") for step in steps]
    bootstrap = next(
        step for step in steps if step.get("name") == "Bootstrap complete release host"
    )
    release_name = next(name for name in names if name and "nightly" in name.lower())

    assert bootstrap["run"] == "sh bootstrap.sh --yes"
    assert bootstrap["env"] == {"CAPSEM_SKIP_ASSET_CHECK": "1"}
    assert "continue-on-error" not in bootstrap
    assert names.index("Bootstrap complete release host") < names.index(release_name)
    assert_unmasked_step(
        "release-nightly.yaml", workflow, job_name, "Bootstrap complete release host"
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
    monkeypatch.setattr("capsem_builder.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("shutil.which", lambda _name: "/usr/bin/bwrap")
    monkeypatch.setattr("capsem_builder.gate.sandbox.active", lambda _config: False)

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
    assert wrapped[wrapped.index("--bind") : wrapped.index("--bind") + 3] == ("--bind", "/", "/")
    assert wrapped[wrapped.index("--dev-bind") : wrapped.index("--dev-bind") + 3] == (
        "--dev-bind",
        "/dev",
        "/dev",
    )
    assert wrapped[-3:] == ("python3", "-c", "print('inside')")


@pytest.mark.parametrize(
    "linux_args",
    [
        ("--die-with-parent", "--new-session", "--bind", "/", "/"),
        ("--unshare-net", "--new-session", "--bind", "/", "/"),
        ("--unshare-net", "--die-with-parent", "--new-session"),
        ("--unshare-net", "--die-with-parent", "--new-session", "--bind", "/", "/"),
    ],
)
def test_linux_sandbox_config_cannot_remove_an_enforcement_property(
    linux_args: tuple[str, ...],
) -> None:
    raw = CONFIG.sandbox.model_dump()
    raw["linux_args"] = linux_args

    with pytest.raises(ValidationError):
        SandboxConfig.model_validate(raw)


@pytest.mark.parametrize(
    ("field", "value"),
    (
        ("linux_probe_loopback_host", "1.1.1.1"),
        ("linux_probe_egress_host", "127.0.0.1"),
        ("linux_probe_egress_port", 65536),
        ("linux_hosted_repair_command", ("sudo", "sh", "-c")),
        ("linux_hosted_userns_sysctl", "net.ipv4.ip_forward"),
        ("linux_hosted_userns_required_value", 0),
        ("linux_hosted_userns_repair_value", 1),
    ),
)
def test_linux_sandbox_config_cannot_widen_the_hosted_repair(field: str, value: object) -> None:
    raw = CONFIG.sandbox.model_dump()
    raw[field] = value

    with pytest.raises(ValidationError):
        SandboxConfig.model_validate(raw)


def test_linux_report_mode_refuses_to_claim_unimplemented_observation(monkeypatch) -> None:
    monkeypatch.setattr("capsem_builder.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("capsem_builder.gate.sandbox.active", lambda _config: False)

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
def test_linux_kernel_boundary_preserves_usable_devices() -> None:
    probe = """
import os

with open(os.devnull, "wb") as sink:
    sink.write(b"gate device probe")
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

    source = (PROJECT_ROOT / "build_system" / "builder" / "gate" / "command.py").read_text(encoding="utf-8")
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
    from capsem_builder.gate.command import GateCommand
    from helpers.gate import built_command

    # Through the helper, which is the one place that knows importing `cli` is
    # what fills the registry -- spelling that import here needs a suppression
    # for a name nothing reads.
    assert built_command(PROJECT_ROOT, "runs", (("limit", None),)).sandboxed == sandbox.OFF
    assert GateCommand.sandboxed == sandbox.OFF
    assert sandbox.mode(sandbox.OFF, None) == sandbox.OFF
    # And an explicit request wins over the declaration, which is what makes
    # report mode a measurement rather than an edit.
    assert sandbox.mode(sandbox.OFF, sandbox.REPORT) == sandbox.REPORT


def test_sandbox_mode_rejects_untyped_callers_at_runtime() -> None:
    """A dynamic string must not cross the same closed seam Ty protects."""
    from typing import Any, cast

    dynamic_mode = cast(Any, sandbox.mode)
    with pytest.raises(TypeError, match="SandboxMode enum"):
        dynamic_mode("off", None)


def test_an_outside_action_uses_only_the_capability_runner() -> None:
    ordinary = RecordingRunner(PROJECT_ROOT)
    capability = RecordingRunner(PROJECT_ROOT)
    variable = CONFIG.environment.command_sandbox_mode

    context = Context(
        ordinary,
        CONFIG,
        outside_runner=capability,
        env={variable: sandbox.ENFORCE.value},
    )
    Run(
        ("python3", "-c", "print('networked')"),
        env={variable: sandbox.REPORT.value},
        outside_sandbox=True,
    ).perform(context)
    Script("scripts/networked.py", outside_sandbox=True).perform(context)

    assert ordinary.commands == []
    assert len(capability.commands) == 2
    assert all(command.env[variable] == "" for command in capability.commands)


def test_an_outside_wrapper_moves_an_opaque_materializer_only() -> None:
    from capsem_builder.gate.outside import Outside

    ordinary = RecordingRunner(PROJECT_ROOT)
    capability = RecordingRunner(PROJECT_ROOT)
    context = Context(ordinary, CONFIG, outside_runner=capability)

    Outside(Run(("docker", "pull", "example@sha256:" + "0" * 64))).perform(context)

    assert ordinary.commands == []
    assert [command.argv for command in capability.commands] == [
        ("docker", "pull", "example@sha256:" + "0" * 64)
    ]


def test_parallel_egress_callers_queue_before_the_single_command_broker(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    response = b'{"ok":true}'
    state_lock = threading.Lock()
    active = 0
    maximum_active = 0

    class FakeSocket:
        def __init__(self, *_args) -> None:
            self._parts = [len(response).to_bytes(8, "big"), response]

        def __enter__(self):
            return self

        def __exit__(self, *_args) -> None:
            nonlocal active
            with state_lock:
                active -= 1

        def connect(self, _endpoint: str) -> None:
            nonlocal active, maximum_active
            with state_lock:
                active += 1
                maximum_active = max(maximum_active, active)
            time.sleep(0.02)

        def sendall(self, _payload: bytes) -> None:
            pass

        def recv(self, _length: int) -> bytes:
            return self._parts.pop(0)

    monkeypatch.setattr(egress.socket, "socket", FakeSocket)
    runner = egress.EgressRunner(
        PROJECT_ROOT,
        endpoint=Path("/unused/egress.sock"),
        token="test-token",
        maximum=1024,
    )
    start = threading.Barrier(6)
    failures: list[Exception] = []

    def request() -> None:
        try:
            start.wait()
            runner._request({"op": "probe"})
        except Exception as error:  # pragma: no cover - asserted below
            failures.append(error)

    threads = [threading.Thread(target=request) for _ in range(6)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()

    assert failures == []
    assert maximum_active == 1


@pytest.mark.parametrize(
    ("name", "args"),
    [
        ("test-fast", ()),
        ("test-static", ()),
        ("candidate", ()),
        ("release-binaries", (("channel", "nightly"),)),
        (
            "release-profile",
            (("channel", "nightly"), ("profile", "code")),
        ),
    ],
)
def test_only_named_dependency_inputs_cross_the_fast_gate_network_boundary(
    name: str, args: tuple[tuple[str, str], ...]
) -> None:
    """Mutable advisory services are narrow exceptions, not a wider gate.

    Mutable advisory queries, digest-authorized distributions, and exact
    dependency-image materializers are the exceptions. Every compiler and
    test action stays inside the loopback-only namespace.
    """
    from helpers.gate import built_command

    plan = built_command(PROJECT_ROOT, name, args)._describe()
    marked = {
        candidate.label
        for candidate in plan.steps
        if any("[outside kernel sandbox]" in line for line in candidate.render())
    }

    if name.startswith("release-"):
        expected = {"source.remote-main", "source.publish-ref", "release"}
        if name == "release-binaries":
            expected.add("channel-source")
    else:
        expected = ONLINE_FAST if name != "test-static" else set()
    if name not in {"test-fast", "release-binaries", "release-profile"}:
        expected |= {
            "host-image",
            "install.materialize",
            "static.guest-builder",
            "static.toolchain.ort",
        }
    assert marked >= expected
    assert not {
        label
        for label in marked
        if label.startswith(("fast.", "static.")) and label not in expected
    }


@pytest.mark.parametrize(
    ("name", "args"),
    [
        ("test-fast", ()),
        ("candidate", ()),
        ("release-binaries", (("channel", "nightly"),)),
        (
            "release-profile",
            (("channel", "nightly"), ("profile", "code")),
        ),
    ],
)
def test_every_fast_gate_entrypoint_prepares_one_scoped_egress_capability(
    name: str,
    args: tuple[tuple[str, str], ...],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from helpers.gate import built_command

    prepared: list[GateConfig] = []
    monkeypatch.setattr("capsem_builder.gate.host.system", lambda: "Linux")
    monkeypatch.setattr("shutil.which", lambda _name: "/usr/bin/bwrap")
    monkeypatch.setattr("capsem_builder.gate.sandbox.active", lambda _config: False)
    monkeypatch.setattr(
        "capsem_builder.gate.sandbox.prepare_egress", lambda config: prepared.append(config)
    )

    replacement = built_command(PROJECT_ROOT, name, args).reexec()

    assert replacement is not None
    assert replacement[0] == CONFIG.sandbox.linux_command
    assert prepared == [CONFIG]


@linux_only
@unnested_only
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


@linux_only
@unnested_only
def test_cancelling_the_outer_runner_reaps_the_bubblewrap_session(tmp_path: Path) -> None:
    """Bubblewrap's new session remains owned through its die-with-parent rail."""
    pid_file = tmp_path / "sandboxed-pid"
    probe = (
        "import os,signal,sys; signal.alarm(5); "
        "open(sys.argv[1],'w').write(str(os.getpid())); signal.pause()"
    )
    wrapped = sandbox.linux_wrap(CONFIG, (sys.executable, "-c", probe, str(pid_file)))
    policy = StopPolicy(
        grace_seconds=CONFIG.execution.cancellation_poll_seconds,
        poll_seconds=CONFIG.execution.cancellation_poll_seconds,
    )

    with cancellation.cancellable() as flag:

        def cancel_when_started() -> None:
            deadline = time.monotonic() + 2
            while not pid_file.is_file() and time.monotonic() < deadline:
                time.sleep(0.01)
            flag.set()

        trigger = threading.Thread(target=cancel_when_started, daemon=True)
        trigger.start()
        with pytest.raises(cancellation.Cancelled):
            Runner(PROJECT_ROOT, stop_policy=policy).run(wrapped)
        trigger.join(timeout=2)

    pid = int(pid_file.read_text())
    deadline = time.monotonic() + 2
    while time.monotonic() < deadline:
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            break
        time.sleep(0.01)
    else:
        pytest.fail(f"sandboxed process {pid} survived cancellation")
