"""Containers, with the flag choices made once.

`docker exec` grew a different set of flags at each of its twenty call sites,
each re-deciding whether to pass `-u capsem`, whether to wrap the command in
`bash -c`, whether to `cd /src` first, and how deeply to escape the quotes --
which is why several ended up three backslashes deep around a single path, and
why one of them did not change directory at all.

The disk budget those containers consume is `storage.py`.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from .errors import GateError
from .proc import Runner


@dataclass(frozen=True)
class Mount:
    """A bind mount or named volume, in `-v` order.

    Refuses the checkout. `-v <repo_root>:/src` let a host step churning
    hardlinks and a container reading the same inodes over virtiofs share a
    filesystem neither declared, which killed a release run with an
    intermittent `Permission denied` on a file that was `0644` before and
    after. Containers get their source copied into an image instead, so there
    is no mount to police.
    """

    source: str
    target: str
    options: str = ""

    #: Set only by `unmigrated`. A mount of the checkout is a defect; until the
    #: four modules that still do it are converted to `COPY`, each one says so
    #: at its call site rather than the guard being switched off globally.
    legacy: bool = False

    @classmethod
    def unmigrated(cls, source: str, target: str, options: str = "") -> Mount:
        """A checkout mount that has not been converted to an image copy yet.

        Deliberately ugly and deliberately greppable. `tests/
        test_gate_docker_boundary.py` counts these and refuses a new one, so
        the list can only shrink.
        """
        return cls(source=source, target=target, options=options, legacy=True)

    def __post_init__(self) -> None:
        if self.legacy:
            return
        # The checkout this package was imported from -- the same derivation
        # `sourcestate.gate_source()` uses, because a `Mount` is constructed
        # before any config is in hand and asking for one would put the check
        # back at the call sites it exists to remove.
        root = Path(__file__).resolve().parents[3]
        # Docker's own rule: a source with no separator is a *named volume*,
        # not a path. Resolving one relative to the cwd puts it inside the
        # checkout and refuses every legitimate cache volume.
        if "/" not in self.source:
            return
        try:
            candidate = Path(self.source).resolve()
        except (OSError, ValueError):
            return
        if candidate == root or root in candidate.parents:
            raise GateError(
                f"{self.source} is inside the checkout: a container that mounts the "
                "working tree shares inodes with every host step, which is a race "
                "no declaration can constrain. COPY the source into the image."
            )

    def __str__(self) -> str:
        return f"{self.source}:{self.target}" + (f":{self.options}" if self.options else "")


class Docker:
    """Container operations, with the flag choices made once."""

    def __init__(self, runner: Runner) -> None:
        self._runner = runner

    # -- lifecycle ---------------------------------------------------------

    def remove(self, container: str) -> None:
        """Detach a container if it exists.

        Deliberately best-effort and deliberately unconditional: a stable
        container name plus a preemptive removal is what lets a run recover
        from a predecessor that died before its own cleanup.
        """
        self._runner.succeeds(["docker", "rm", "-f", container])

    def run_detached(
        self,
        *,
        name: str,
        image: str,
        command: list[str],
        network: str,
        options: list[str] | None = None,
        mounts: list[Mount] | None = None,
    ) -> None:
        """Start a container in the background.

        `network` has no default on purpose. Nothing in the gate passed
        `--network` at all, so every container had outbound access and several
        fetched mid-run -- which is the difference between a gate that proves
        a build reproduces and one that proves it reproduces today.
        """
        argv = ["docker", "run", "-d", "--name", name, "--network", network, *(options or [])]
        for mount in mounts or []:
            argv += ["-v", str(mount)]
        argv += [image, *command]
        self._runner.run(argv)

    def run_once(
        self,
        *,
        image: str,
        command: list[str],
        network: str,
        options: list[str] | None = None,
        mounts: list[Mount] | None = None,
        check: bool = True,
    ) -> None:
        """Run a container to completion and remove it."""
        argv = ["docker", "run", "--rm", "--network", network, *(options or [])]
        for mount in mounts or []:
            argv += ["-v", str(mount)]
        argv += [image, *command]
        self._runner.run(argv, check=check)

    # -- images ------------------------------------------------------------

    def build(
        self, *, tag: str, dockerfile: str, context: str, args: list[str] | None = None
    ) -> None:
        """Build an image. The context streams from the CLI, so it does not
        have to be visible inside the Lima VM the way a bind mount does."""
        argv = ["docker", "build", "-t", tag, "-f", dockerfile]
        for value in args or []:
            argv += ["--build-arg", value]
        argv.append(context)
        self._runner.run(argv)

    def image_exists(self, tag: str) -> bool:
        return self._runner.succeeds(["docker", "image", "inspect", tag])

    def image_id(self, tag: str) -> str:
        """The exact image a mutable tag currently names.

        `:latest` is a pointer, so "the same tag" is a different image after
        every rebuild. Anything that keys a cache off a parent image has to key
        it off this, or it reuses work built against an image that no longer
        exists under that name.
        """
        found = self._runner.capture(
            ["docker", "image", "inspect", "--format", "{{.Id}}", tag]
        ).strip()
        if not found:
            raise GateError(f"docker has no image tagged {tag}, so nothing can be keyed by it")
        return found

    # -- extraction --------------------------------------------------------

    def create(
        self,
        *,
        name: str,
        image: str,
        command: list[str],
        network: str,
        env: dict[str, str] | None = None,
    ) -> None:
        """Create a container without starting it, so `copy_out` has something
        to read. `--rm` and `docker cp` are mutually exclusive: a removed
        container has nothing left to copy from, which is why extraction
        cannot reuse `run_once`."""
        argv = ["docker", "create", "--name", name, "--network", network]
        for key, value in (env or {}).items():
            argv += ["-e", f"{key}={value}"]
        argv += [image, *command]
        self._runner.run(argv)

    def start(self, container: str) -> None:
        self._runner.run(["docker", "start", "-a", container])

    def copy_out(self, container: str, source: str, destination: str) -> None:
        """Take bytes out of a container without a writable mount."""
        self._runner.run(["docker", "cp", f"{container}:{source}", destination])

    # -- exec --------------------------------------------------------------

    def _exec_argv(
        self,
        container: str,
        argv: list[str],
        *,
        user: str | None,
        env: dict[str, str] | None,
        detach: bool,
    ) -> list[str]:
        prefix = ["docker", "exec"]
        if detach:
            prefix.append("-d")
        if user:
            prefix += ["-u", user]
        for name, value in (env or {}).items():
            prefix += ["-e", f"{name}={value}"]
        return [*prefix, container, *argv]

    def exec(
        self,
        container: str,
        argv: list[str],
        *,
        user: str | None = None,
        env: dict[str, str] | None = None,
        detach: bool = False,
        check: bool = True,
    ) -> int:
        return self._runner.run(
            self._exec_argv(container, argv, user=user, env=env, detach=detach),
            check=check,
        )

    def capture(
        self,
        container: str,
        argv: list[str],
        *,
        user: str | None = None,
        env: dict[str, str] | None = None,
    ) -> str:
        return self._runner.capture(
            self._exec_argv(container, argv, user=user, env=env, detach=False)
        )

    def succeeds(
        self,
        container: str,
        argv: list[str],
        *,
        user: str | None = None,
    ) -> bool:
        return self._runner.succeeds(
            self._exec_argv(container, argv, user=user, env=None, detach=False)
        )

    def shell(
        self,
        container: str,
        script: str,
        *,
        user: str | None = None,
        env: dict[str, str] | None = None,
        detach: bool = False,
        check: bool = True,
        cwd: str | None = None,
    ) -> int:
        """Run a shell fragment inside the container.

        `cwd` exists because nearly every call site began with `cd /src && `,
        and one of them did not -- which is the kind of difference that is
        invisible in a wall of escaped quotes.
        """
        body = f"cd {cwd} && {script}" if cwd else script
        return self.exec(
            container, ["bash", "-c", body], user=user, env=env, detach=detach, check=check
        )

    def shell_capture(
        self,
        container: str,
        script: str,
        *,
        user: str | None = None,
        cwd: str | None = None,
    ) -> str:
        body = f"cd {cwd} && {script}" if cwd else script
        return self.capture(container, ["bash", "-c", body], user=user)

    def exists(self, path: str, container: str, *, user: str | None = None) -> bool:
        return self.succeeds(container, ["test", "-f", path], user=user)


def container_path(root: Path, host_path: Path, *, mount: str) -> str:
    """Where a checkout path appears inside a container that bind-mounts it."""
    host_path = Path(host_path)
    root = Path(root)
    try:
        relative = host_path.relative_to(root)
    except ValueError:
        raise GateError(f"{host_path} is outside the mounted checkout {root}") from None
    return f"{mount}/{relative}"
