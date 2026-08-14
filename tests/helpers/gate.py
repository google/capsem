"""A `Runner` that records commands instead of running them.

The defects this refactor exists to prevent are ordering defects: a manifest
URL consumed before anything wrote the manifest, a container reused after it
was removed, storage released before the rail that needed it finished. None of
those is visible in a single command, so a test that stubs `subprocess` call by
call cannot see them either.

`RecordingRunner` keeps the whole sequence, and `index_of` turns "A must
precede B" into an assertion.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
import tempfile
from collections.abc import Iterable, Iterator
from contextlib import contextmanager, suppress
from functools import cache
from pathlib import Path
from typing import TextIO

from capsem.gate.invocation import Command
from capsem.gate.proc import Runner
from capsem.gate.runlogschema import OutputSpan

PROJECT_ROOT = Path(__file__).resolve().parents[2]

#: The one probe whose *answer* decides which commands the plan goes on to
#: issue, rather than being recorded and never read.
#:
#: `docker_git_metadata_mount` skips this entirely when `.git` is a directory,
#: so an ordinary checkout never asks. A linked worktree carries a `.git` file
#: instead, asks, and gets "" back from a recorder that answers nothing -- an
#: unresolvable common dir, which the gate correctly refuses to build against.
#: The plan then dies at `package.<arch>.build`, and every ordering contract
#: about a command issued at or after that point fails for want of a git
#: answer rather than for anything the contract is about.
#:
#: Answered truthfully, from the real repository, because a wrong path here
#: would be a `-v` mount these contracts then assert against.
GIT_COMMON_DIR_PROBE = "--git-common-dir"

#: The other probe whose answer decides what follows. `base_tag` hashes the
#: current id of `capsem-host-builder:latest` into the Linux parity base
#: image's identity, because `:latest` is a pointer and a rebuilt parent is a
#: different image under the same name. Unanswered, the lane refuses before it
#: issues anything, and every contract about what it issues fails for want of a
#: docker answer rather than for what the contract is about.
#:
#: Canned rather than read from the daemon: a recorder must not need Docker
#: running, and the id only shifts the digest -- what these contracts assert is
#: the shape of what gets issued. A test that needs the resulting tag composes
#: it from the same recorder.
IMAGE_ID_PROBE = "{{.Id}}"
IMAGE_PLATFORM_ID_PROBE = "{{.Os}}/{{.Architecture}}"
IMAGE_LABEL_PROBE = "index .Config.Labels"
IMAGE_REPOSITORY_DIGEST_PROBE = "{{json .RepoDigests}}"
RECORDED_IMAGE_ID = "sha256:" + "0" * 64


def _image_repository(reference: str) -> str:
    name = reference.split("@", 1)[0]
    if name.rfind(":") > name.rfind("/"):
        name = name.rsplit(":", 1)[0]
    return name


def _recorded_image_platform(root: Path, reference: str) -> str:
    """Answer an identity probe from the checked-in architecture authority."""
    from capsem.builder import guestbuilder
    from capsem.gate import config as gate_config
    from capsem.gate import host, imagebases

    config_root = root if (root / "config/gate.toml").is_file() else PROJECT_ROOT
    config = gate_config.load(config_root)
    host_platform = config.arch(host.machine()).docker_platform
    if reference.startswith(
        (
            "capsem-host-builder",
            "capsem-package-builder-",
            "capsem-install-builder",
            "capsem-guest-rust-",
            "capsem-asset-tools-",
        )
    ):
        return host_platform
    try:
        build = imagebases.build_config(config)
    except OSError:
        build = None
    if build is not None:
        for name, arch in build.architectures.items():
            if reference == arch.base_image:
                return arch.docker_platform
            resolved = guestbuilder.environment(build, name)
            if reference == resolved.base_image:
                return resolved.docker_platform
            if reference == guestbuilder.image_tag(build, name, config_root):
                return resolved.docker_platform
    for name, arch in config.architectures.items():
        if name in reference or any(alias in reference for alias in arch.aliases):
            return arch.docker_platform
    return host_platform


def recorded_image_identity(
    root: Path, reference: str, *, image_id: str = RECORDED_IMAGE_ID
) -> str:
    """Return the complete portable identity shape used by Docker recorders."""
    return f"{_recorded_image_platform(root, reference)}\t{image_id}"


@cache
def _git_common_dir(root: Path) -> str:
    """What the probe would really have answered in this checkout."""
    found = subprocess.run(
        ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    return found.stdout.strip()


@cache
def _cargo_tool_probe_replies(root: Path) -> dict[tuple[str, ...], str]:
    """Config-owned answers for plan-only toolchain probes.

    A recorder must not inherit whatever Cargo tools happen to be installed on
    the test host. These answers let composition tests cross the same exact
    version boundary without copying the versions into test infrastructure.
    """
    from capsem.gate import config as gate_config

    config_root = root if (root / "config/gate.toml").is_file() else PROJECT_ROOT
    settings = gate_config.load(config_root).toolchain
    return {crate.probe: crate.expected for crate in settings.crates}


@cache
def _cargo_tool_probe_executables() -> frozenset[str]:
    """Executables that can be a config-owned Cargo tool version probe."""
    return frozenset(probe[0] for probe in _cargo_tool_probe_replies(PROJECT_ROOT))


class RecordingRunner(Runner):
    """Records every command; answers with canned output.

    `replies` maps a substring of the rendered command to the stdout that
    command should produce. `failures` does the same for exit statuses, so a
    test can make one step fail and assert what the gate does next.
    """

    def __init__(
        self,
        root: Path,
        *,
        replies: dict[str, str] | None = None,
        failures: Iterable[str] = (),
        stream: TextIO | None = None,
    ) -> None:
        super().__init__(root, stream=stream)
        self.commands: list[Command] = []
        self.notes: list[str] = []
        self._replies = dict(replies or {})
        self._failures = tuple(failures)

    # -- Runner overrides --------------------------------------------------

    def execute(self, command: Command) -> subprocess.CompletedProcess[str]:
        self.commands.append(command)
        rendered = str(command)
        status = 1 if any(marker in rendered for marker in self._failures) else 0
        stdout = ""
        for marker, reply in self._replies.items():
            if marker in rendered:
                stdout = reply
                break
        else:
            if GIT_COMMON_DIR_PROBE in rendered:
                stdout = _git_common_dir(self.root)
            elif IMAGE_PLATFORM_ID_PROBE in rendered:
                stdout = recorded_image_identity(self.root, command.argv[-1])
            elif IMAGE_ID_PROBE in rendered:
                stdout = RECORDED_IMAGE_ID
            elif IMAGE_LABEL_PROBE in rendered:
                if "org.capsem.host-builder.input-key" in rendered:
                    from capsem.gate import config as gate_config
                    from capsem.gate import hostimage

                    config_root = (
                        self.root if (self.root / "config/gate.toml").is_file() else PROJECT_ROOT
                    )
                    stdout = hostimage.input_key(gate_config.load(config_root))
                else:
                    stdout = command.argv[-1]
            elif IMAGE_REPOSITORY_DIGEST_PROBE in rendered:
                repository = _image_repository(command.argv[-1])
                stdout = f'["{repository}@{RECORDED_IMAGE_ID}"]'
            elif (
                command.argv
                and command.argv[0] in _cargo_tool_probe_executables()
                and command.argv in _cargo_tool_probe_replies(self.root)
            ):
                stdout = _cargo_tool_probe_replies(self.root)[command.argv]
        return subprocess.CompletedProcess(
            args=list(command.argv), returncode=status, stdout=stdout, stderr=""
        )

    def launch(
        self,
        argv: Iterable[str],
        *,
        env: dict[str, str] | None = None,
        cwd: Path | None = None,
        secret_env: frozenset[str] = frozenset(),
    ) -> int:
        """Record a detached start instead of spawning one.

        Inherited from `Runner` until now, so any test that reached a `Launch`
        really did `Popen` a daemon -- which on a checkout without the binary
        built is a `FileNotFoundError` from a destructor, and on one with it is
        a stray process.
        """
        self.commands.append(
            Command(
                argv=tuple(str(part) for part in argv),
                cwd=cwd,
                env=dict(env or {}),
                secret_env=secret_env,
            )
        )
        return 424242

    def step(self, message: str) -> None:
        self.notes.append(message)

    def note(self, message: str) -> None:
        self.notes.append(message)

    def fail_on(self, *markers: str) -> None:
        """Change what fails partway through, for before/after checks."""
        self._failures = markers

    # -- assertions --------------------------------------------------------

    @property
    def rendered(self) -> list[str]:
        return [str(command) for command in self.commands]

    def index_of(self, pattern: str) -> int:
        """Position of the first command matching `pattern` as a regex.

        Fails loudly rather than returning -1: a missing command and a
        mis-ordered one are different bugs, and `assert a < b` on a -1 quietly
        reports the wrong one.
        """
        expression = re.compile(pattern)
        for position, rendered in enumerate(self.rendered):
            if expression.search(rendered):
                return position
        raise AssertionError(
            f"no command matched {pattern!r}; ran:\n  " + "\n  ".join(self.rendered)
        )

    def last_index_of(self, pattern: str) -> int:
        """Position of the *last* match.

        Some commands legitimately run twice -- `docker rm -f` both clears a
        predecessor and tears this run down -- and asserting on the first
        occurrence would prove the wrong one happened.
        """
        expression = re.compile(pattern)
        for position in reversed(range(len(self.rendered))):
            if expression.search(self.rendered[position]):
                return position
        raise AssertionError(
            f"no command matched {pattern!r}; ran:\n  " + "\n  ".join(self.rendered)
        )

    def matching(self, pattern: str) -> list[str]:
        expression = re.compile(pattern)
        return [line for line in self.rendered if expression.search(line)]

    def ran(self, pattern: str) -> bool:
        return bool(self.matching(pattern))

    def assert_order(self, *patterns: str) -> None:
        """Assert the given commands ran, in the given order."""
        positions = [self.index_of(pattern) for pattern in patterns]
        assert positions == sorted(positions), (
            "commands ran out of order: "
            + ", ".join(f"{p}@{i}" for p, i in zip(patterns, positions, strict=False))
            + "\nran:\n  "
            + "\n  ".join(self.rendered)
        )


class RecordingJournal:
    """A `Journal` that keeps what was reported, so a test can read it back.

    Shared rather than re-declared per test file: three of them had grown their
    own, and each widening of the protocol had to find all three.
    """

    def __init__(self) -> None:
        self.run_id = "recording"
        self.notes: list[str] = []
        self.artifacts: list[tuple[Path, str, int]] = []
        self.steps: list[str] = []
        self.actions: list[str] = []
        self.shapes: list[tuple[tuple[str, ...], tuple[tuple[str, str], ...]]] = []
        self.execs: list[dict] = []
        self.launches: list[dict] = []
        self.skips: list[str] = []
        self.carries: list[str] = []
        self.waits: list[tuple[str, float, float, float]] = []

    def note(self, message: str) -> None:
        self.notes.append(message)

    def exec(
        self,
        argv: tuple[str, ...],
        *,
        cwd: str,
        env: dict[str, str],
        exit: int,
        duration_ms: float,
        output: OutputSpan | None = None,
    ) -> None:
        self.execs.append(
            {
                "argv": argv,
                "cwd": cwd,
                "env": env,
                "exit": exit,
                "duration_ms": duration_ms,
                "output": output,
            }
        )

    def launch(
        self,
        argv: tuple[str, ...],
        *,
        cwd: str,
        env: dict[str, str],
        pid: int,
        duration_ms: float,
    ) -> None:
        self.launches.append(
            {"argv": argv, "cwd": cwd, "env": env, "pid": pid, "duration_ms": duration_ms}
        )

    def artifact(self, path: Path, *, digest: str, size: int) -> None:
        self.artifacts.append((path, digest, size))

    def shape(self, steps: tuple[str, ...], edges: tuple[tuple[str, str], ...]) -> None:
        self.shapes.append((steps, edges))

    def skipped(self, label: str) -> None:
        self.skips.append(label)

    def carried(self, label: str) -> None:
        """Separately from `skips`, because they are separate claims.

        A skipped step never ran because something before it failed; a carried
        step was proved by an earlier run and deliberately not repeated. A
        double that folded them together would let a test assert "not skipped"
        about a run that carried half its graph.
        """
        self.carries.append(label)

    def waited(
        self, label: str, *, dependency_ms: float, resource_ms: float, execution_ms: float
    ) -> None:
        self.waits.append((label, dependency_ms, resource_ms, execution_ms))

    def step_output(self) -> Path | None:
        """Nothing: a recording journal keeps events, not bytes."""
        return None

    @contextmanager
    def step(self, step) -> Iterator[None]:
        self.steps.append(step.label)
        yield

    @contextmanager
    def action(self, action) -> Iterator[None]:
        self.actions.append(action.render())
        yield


# ---------------------------------------------------------------------------
# Reading a contract off the gate instead of off a recipe
# ---------------------------------------------------------------------------
#
# Dozens of contracts asserted against `justfile` text, because that is where
# the work was. The recipes are one-line dispatches now and the work is a plan,
# so the same claims are read by running the plan against a recording runner
# and asking what it would have issued.
#
# Cached: building and walking a plan costs seconds, and a suite that asks the
# same question thirty times should pay once.

#: Running the whole gate's plan stops at the first step that needs a real
#: machine, so "what does the gate run" is gathered per module instead -- the
#: same work, reached without one failure hiding the rest.
WHOLE_GATE: tuple[tuple[str, dict[str, object]], ...] = (
    ("candidate", {}),
    ("test-fast", {}),
    ("test-static", {}),
    ("test-artifacts", {}),
    ("test-functional", {}),
    ("test-glowup", {}),
    ("cross-compile", {"arch": "arm64"}),
    ("cross-compile", {"arch": "x86_64"}),
    ("linux-rust", {}),
    ("host-sbom", {}),
    ("install", {}),
    ("assets", {}),
)


def _built(root: Path, name: str, args: tuple[tuple[str, object], ...], qualification=None):
    import argparse

    from capsem.gate import cli  # noqa: F401 - importing registers every command
    from capsem.gate.command import GateCommand

    values = dict(args)
    if name in {"release-binaries", "release-profile"}:
        from capsem.gate.sourcecommit import SourceCommit

        values.setdefault("source_commit", SourceCommit("0" * 40))
    return GateCommand.registry[name](
        RecordingRunner(root),
        argparse.Namespace(dry_run=False, graph=False, timing=False, **values),
        qualification=qualification,
    )


def built_command(
    root: Path, name: str, args: tuple[tuple[str, object], ...] = (), qualification=None
):
    """One command against a recording runner, for a test that drives it itself.

    Public because a test that needs to run a plan against a *modified* config
    -- pointing an output somewhere private so it does not collide with the
    gate running the suite -- cannot go through `gate_issued`, which builds its
    own `Context` from the real one.
    """
    return _built(root, name, args, qualification)


def gate_plan(name: str = "candidate", root: Path | None = None, qualification=None):
    """A command's plan, built but not run -- for asserting on its edges.

    Deliberately not cached. A plan is a mutable object with an environment in
    its inputs, and a cache keyed on the name alone hands two callers the same
    one -- so a test that set a release variable and asked again got the local
    lane's plan and asserted happily against it. Building one costs
    milliseconds; the answers it hides cost hours.
    """
    return _built(root or PROJECT_ROOT, name, (), qualification)._describe()


def gate_labels(name: str = "candidate", root: Path | None = None) -> tuple[str, ...]:
    """Every step of a command's plan, in an order the graph permits."""
    return tuple(gate_plan(name, root).labels)


def gate_issued(
    name: str, args: tuple[tuple[str, object], ...] = (), root: Path | None = None
) -> str:
    """Every command one gate command would actually run, with real argv.

    The plan is *run* against a recording runner rather than described: much of
    this work is still behind `Call`, which renders as prose, and these
    contracts are about the arguments underneath.
    """
    source = root or PROJECT_ROOT
    with _inspection_checkout(source) as checkout:
        return _gate_issued_from(checkout, name, args)


@contextmanager
def _inspection_checkout(source: Path) -> Iterator[Path]:
    """Give opaque plan callbacks an expendable checkout to mutate.

    A recording runner makes subprocess actions inert, but a ``Call`` executes
    Python in-process and may legitimately clear or rebuild output.  Plan
    introspection therefore needs the same source isolation as a real gate,
    not the live tree whose outputs a concurrent test may be consuming.
    """
    from unittest.mock import patch

    from capsem.gate import config as gate_config
    from capsem.gate import host, snapshot

    with tempfile.TemporaryDirectory(
        prefix=".capsem-gate-inspect-", dir=source.parent
    ) as temporary:
        checkout = Path(temporary) / "checkout"
        # Plan contracts deliberately monkeypatch the gate's host to render a
        # Darwin plan on Linux.  The copy itself still runs on this kernel, so
        # its clonefile choice must use the real interpreter platform.
        with patch.object(host, "on_macos", return_value=sys.platform == "darwin"):
            snapshot.populate(source, checkout, gate_config.load(source))
        _seed_observed_source(checkout)
        yield checkout


def _seed_observed_source(checkout: Path) -> None:
    """Materialize source prerequisites only inside the expendable reader.

    ``observing=True`` must keep source.record from touching the live gate's
    receipt, but later opaque callbacks legitimately require the frozen source
    product. Seed that product in the isolated checkout so observation can
    reach those callbacks without weakening either production requirement.
    """
    from capsem.gate import config as gate_config
    from capsem.gate import snapshot, sourcecapture
    from capsem.gate.filesystem import write_text

    config = gate_config.load(checkout)
    digest = sourcecapture.SourceDigest(snapshot.digest(checkout, config))
    write_text(
        config.path(config.candidate.source_state_file),
        json.dumps({"digest": digest}),
    )
    sourcecapture.capture(config, expected=digest)


def _gate_issued_from(root: Path, name: str, args: tuple[tuple[str, object], ...]) -> str:
    """Run one plan against a recorder inside an already-isolated checkout."""
    from capsem.gate import config as gate_config
    from capsem.gate.context import Context

    command = _built(root, name, args)
    runner = command._runner
    try:
        plan = command._describe()
    except Exception as exc:
        return f"<plan for {name} unavailable: {exc}>"

    rendered = plan.describe()
    # A step that needs a machine fails here; what it issued before failing is
    # still the evidence.
    with suppress(Exception):
        plan.run(Context(runner, gate_config.load(root), observing=True))
    return "\n".join([rendered, *runner.rendered, *runner.notes])


def gate_issues(name: str | None = None, root: Path | None = None) -> str:
    """Everything the gate would issue, with real argv.

    `name` reads one command; the default reads the whole gate, which is what a
    contract about "does the gate ever run X" is really asking.

    Cached, unlike `gate_plan`: this runs twelve module plans and the answer is
    an immutable string. The release state is part of the key rather than
    ambient, because it changes the answer -- which is what a cache with only
    the name in its key was quietly denying.
    """
    from capsem.gate import config as gate_config
    from capsem.gate.qualification import from_environment

    mode = from_environment(gate_config.load(root or PROJECT_ROOT)).mode
    return _issues(name, root, mode)


@cache
def _issues(name: str | None, root: Path | None, mode: object) -> str:
    selection = (
        tuple(entry for entry in WHOLE_GATE if entry[0] == name) if name is not None else WHOLE_GATE
    )
    source = root or PROJECT_ROOT
    with _inspection_checkout(source) as checkout:
        return "\n".join(
            _gate_issued_from(checkout, command, tuple(sorted(args.items())))
            for command, args in selection
        )
