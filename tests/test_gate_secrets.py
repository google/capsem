"""A run directory is safe to attach to a bug report. That has to be true.

`runlog` promises it -- "`exec` records only the environment a command *added*,
never the ambient one, because a release machine's environment holds tokens" --
and the package rail then broke it from the other side. It read the checkout's
real Tauri private key and password out of `private/tauri/` and put both into
`docker run`'s argv as `-e NAME=value`. From there the values reached the
process listing, `run.jsonl`, the step error of any failed build, `run.end`'s
failures, and the summary.

Secrecy is a property of the command here, not of a name somebody remembered to
filter. A declared secret cannot be rendered: not by `str(command)`, not by the
journal, not by the exception a failure raises.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from capsem.gate import config as gate_config
from capsem.gate.errors import GateError
from capsem.gate.funnel import GuardedRunner
from capsem.gate.invocation import Command
from capsem.gate.proc import Runner

PROJECT_ROOT = Path(__file__).resolve().parents[1]
CONFIG = gate_config.load(PROJECT_ROOT)
SIGNING = CONFIG.package.signing

#: What must never appear anywhere. Distinctive enough that a substring search
#: over a whole directory tree is a real assertion.
SENTINEL = "secret-key-bytes-do-not-log-me"
PASSPHRASE = "secret-passphrase-do-not-log-me"

REDACTED = "<redacted>"


class _Recording(Runner):
    """Runs nothing; keeps the exact `Command` it was handed."""

    def __init__(self, root: Path, *, fail: bool = False) -> None:
        super().__init__(root)
        self.commands: list[Command] = []
        self._fail = fail

    def execute(self, command: Command):
        import subprocess

        self.commands.append(command)
        return subprocess.CompletedProcess(
            args=list(command.argv), returncode=1 if self._fail else 0, stdout="", stderr=""
        )

    def launch(self, argv, *, env=None, cwd=None, secret_env=frozenset()) -> int:
        """Recorded, not spawned: the inherited one really starts a daemon."""
        self.commands.append(
            Command(
                argv=tuple(str(part) for part in argv),
                cwd=cwd,
                env=dict(env or {}),
                secret_env=secret_env,
            )
        )
        return 424242


def _secret_command() -> Command:
    return Command(
        argv=("docker", "run", "--rm", "-e", SIGNING.key_variable, "alpine"),
        env={SIGNING.key_variable: SENTINEL, "TARGET_ARCH": "arm64"},
        secret_env=frozenset({SIGNING.key_variable}),
    )


# ---------------------------------------------------------------------------
# Rendering
# ---------------------------------------------------------------------------


def test_a_declared_secret_never_renders_its_value() -> None:
    rendered = str(_secret_command())

    assert SENTINEL not in rendered
    assert f"{SIGNING.key_variable}={REDACTED}" in rendered
    # Everything else stays legible: a redaction that hides the whole command
    # is a redaction nobody can debug against.
    assert "TARGET_ARCH=arm64" in rendered
    assert "docker run --rm" in rendered


def test_a_secret_that_leaked_into_argv_is_redacted_there_too() -> None:
    """Belt and braces, and the shape this actually had.

    The rail built `-e NAME=value` argv. Declaring the name is enough to make
    that spelling safe as well, so a future call site that reintroduces it
    fails closed rather than silently.
    """
    command = Command(
        argv=("docker", "run", "-e", f"{SIGNING.key_variable}={SENTINEL}", "alpine"),
        env={SIGNING.key_variable: SENTINEL},
        secret_env=frozenset({SIGNING.key_variable}),
    )

    assert SENTINEL not in str(command)
    assert SENTINEL not in " ".join(command.evidence_argv)


def test_an_undeclared_command_renders_exactly_as_before() -> None:
    plain = Command(argv=("cargo", "build"), env={"RUSTFLAGS": "-C debuginfo=0"})

    assert str(plain) == "RUSTFLAGS='-C debuginfo=0' cargo build"


# ---------------------------------------------------------------------------
# The failure path, which is where a raw command used to be printed
# ---------------------------------------------------------------------------


def test_a_failing_secret_command_keeps_its_secret_out_of_the_error() -> None:
    runner = _Recording(PROJECT_ROOT, fail=True)
    command = _secret_command()

    with pytest.raises(GateError) as raised:
        runner.run(command.argv, env=command.env, secret_env=command.secret_env, check=True)

    assert SENTINEL not in str(raised.value)
    assert REDACTED in str(raised.value)


# ---------------------------------------------------------------------------
# The journal, which is the file that gets attached
# ---------------------------------------------------------------------------


class _Journal:
    def __init__(self) -> None:
        self.execs: list[dict] = []
        self.launches: list[dict] = []

    def exec(self, argv, *, cwd, env, exit, duration_ms) -> None:
        self.execs.append({"argv": argv, "cwd": cwd, "env": env, "exit": exit})

    def launch(self, argv, *, cwd, env, pid, duration_ms) -> None:
        self.launches.append({"argv": argv, "cwd": cwd, "env": env, "pid": pid})

    def step_output(self):
        """No step, so nowhere to file output -- which is not this test's claim."""
        return


@pytest.mark.parametrize("failing", (False, True))
def test_the_journal_records_the_name_and_never_the_value(failing: bool) -> None:
    journal = _Journal()
    runner = GuardedRunner(_Recording(PROJECT_ROOT, fail=failing), journal=journal)
    command = _secret_command()

    runner.run(command.argv, env=command.env, secret_env=command.secret_env, check=False)

    (recorded,) = journal.execs
    serialized = json.dumps(recorded)
    assert SENTINEL not in serialized
    # The name survives: "which variable was set" is the part worth having.
    assert recorded["env"][SIGNING.key_variable] == REDACTED
    assert recorded["env"]["TARGET_ARCH"] == "arm64"


def test_a_launched_daemon_redacts_the_same_way() -> None:
    journal = _Journal()
    runner = GuardedRunner(_Recording(PROJECT_ROOT), journal=journal)

    runner.launch(
        ["capsem-service"],
        env={SIGNING.password_variable: PASSPHRASE},
        secret_env=frozenset({SIGNING.password_variable}),
    )

    (recorded,) = journal.launches
    assert PASSPHRASE not in json.dumps(recorded)
    assert recorded["env"][SIGNING.password_variable] == REDACTED


# ---------------------------------------------------------------------------
# The rail itself: the argv it builds, and a whole run directory
# ---------------------------------------------------------------------------


def _rail_with_keys(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """A checkout that has real signing material, as a release machine does."""
    directory = tmp_path / SIGNING.directory
    directory.mkdir(parents=True)
    (directory / SIGNING.key).write_text(SENTINEL, encoding="utf-8")
    (directory / SIGNING.password).write_text(PASSPHRASE, encoding="utf-8")
    import shutil

    for name in ("config", "docker"):
        source = PROJECT_ROOT / name
        if source.is_dir():
            shutil.copytree(source, tmp_path / name, dirs_exist_ok=True)
    shutil.copy(PROJECT_ROOT / "rust-toolchain.toml", tmp_path / "rust-toolchain.toml")
    return directory


def test_the_docker_argv_names_the_variable_and_carries_the_value_in_the_child_env(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """`-e NAME`, not `-e NAME=value`.

    Docker forwards the value from its own environment, so the key never
    reaches argv -- and therefore never reaches `ps`, which is readable by
    every user on the machine and which no amount of log redaction covers.
    """
    from capsem.gate.packagerail import PackageRail

    _rail_with_keys(tmp_path, monkeypatch)
    runner = _Recording(tmp_path)
    config = gate_config.load(tmp_path)
    rail = PackageRail(runner, config.arch(next(iter(config.architectures))))

    rail.build()

    (command,) = [c for c in runner.commands if c.argv[0] == "docker"]
    argv = " ".join(command.argv)
    assert SENTINEL not in argv
    assert PASSPHRASE not in argv
    assert f"-e {SIGNING.key_variable} " in argv + " "
    assert f"{SIGNING.key_variable}=" not in argv
    # It still reaches the container, which is the whole point of reading it.
    assert command.env[SIGNING.key_variable] == SENTINEL
    assert command.secret_env == frozenset({SIGNING.key_variable, SIGNING.password_variable})


def test_no_byte_of_a_recorded_run_holds_the_signing_material(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The claim as stated: the directory is safe to attach.

    Asserted over every file the run wrote, because the leak reached four
    different ones and a check per file is a check that misses the fifth.
    """
    from capsem.gate.packagerail import PackageRail
    from capsem.gate.runlog import RunLog

    _rail_with_keys(tmp_path, monkeypatch)
    config = gate_config.load(tmp_path)
    inner = _Recording(tmp_path, fail=True)

    with RunLog.open(config, "cross-compile", argv=("cross-compile",)) as log:
        runner = GuardedRunner(inner, journal=log)
        rail = PackageRail(runner, config.arch(next(iter(config.architectures))))
        with pytest.raises(GateError):
            rail.build()

    written = sorted(path for path in log.directory.rglob("*") if path.is_file())
    assert written, "a run that wrote nothing proves nothing about what it writes"
    for path in written:
        body = path.read_text(encoding="utf-8", errors="replace")
        assert SENTINEL not in body, f"the private key reached {path.name}"
        assert PASSPHRASE not in body, f"the key password reached {path.name}"
