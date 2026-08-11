"""Containers, with the flag choices made once.

`docker exec` grew a different set of flags at each of its twenty call sites,
each re-deciding whether to pass `-u capsem`, whether to wrap the command in
`bash -c`, whether to `cd /src` first, and how deeply to escape the quotes --
which is why several ended up three backslashes deep around a single path, and
why one of them did not change directory at all.

The disk budget those containers consume is `storage.py`.
"""

from __future__ import annotations

from .dockerimage import ImageOperations, require_container_network
from .dockermount import Mount
from .errors import GateError
from .invocation import ConsoleMode
from .proc import Runner


class Docker(ImageOperations):
    """Container operations, with the flag choices made once."""

    def __init__(self, runner: Runner) -> None:
        self._runner = runner

    # -- lifecycle ---------------------------------------------------------

    def remove(self, container: str) -> None:
        """Detach a container if it exists, and take its scratch with it.

        Deliberately best-effort and deliberately unconditional: a stable
        container name plus a preemptive removal is what lets a run recover
        from a predecessor that died before its own cleanup.

        `-v` removes the anonymous volumes `create(scratch=...)` made. Without
        it every package build leaves a 356 MB node_modules volume behind, and
        nothing else would ever collect them -- they have no name to reclaim by.
        """
        self._runner.succeeds(["docker", "rm", "-f", "-v", container])

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
        network = require_container_network(network)
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
        network = require_container_network(network)
        argv = ["docker", "run", "--rm", "--network", network, *(options or [])]
        for mount in mounts or []:
            argv += ["-v", str(mount)]
        argv += [image, *command]
        self._runner.run(argv, check=check)

    def probe(
        self,
        *,
        image: str,
        command: list[str],
        network: str,
        options: tuple[str, ...] = (),
        user: str | None = None,
        env: dict[str, str] | None = None,
        mounts: tuple[Mount, ...] = (),
    ) -> bool:
        """Run a container to completion and report whether it worked.

        For a preflight whose *answer* is the exit status. `run_once` raises,
        which is right for work and wrong for a question -- and a call site
        that wants the answer had to build its own argv to get it, which is how
        the last hand-built `docker run` in the gate outlived the wrapper.
        """
        network = require_container_network(network)
        argv = ["docker", "run", "--rm", "--network", network, *options]
        if user is not None:
            argv += ["-u", user]
        for key, value in (env or {}).items():
            argv += ["-e", f"{key}={value}"]
        argv += [part for mount in mounts for part in ("-v", str(mount))]
        argv += [image, *command]
        return self._runner.succeeds(argv)

    # -- extraction --------------------------------------------------------

    def create(
        self,
        *,
        name: str,
        image: str,
        command: list[str],
        network: str,
        env: dict[str, str] | None = None,
        forward: tuple[str, ...] = (),
        carry: dict[str, str] | None = None,
        mounts: tuple[Mount, ...] = (),
        scratch: tuple[str, ...] = (),
        workdir: str | None = None,
        secret_env: frozenset[str] = frozenset(),
    ) -> None:
        """Create a container without starting it, so `copy_out` has something
        to read. `--rm` and `docker cp` are mutually exclusive: a removed
        container has nothing left to copy from, which is why extraction
        cannot reuse `run_once`.

        Two ways to hand a variable in, and the difference is the whole point.
        `env` writes `-e NAME=value` into argv, for values that are not
        secrets. `forward` writes `-e NAME` and leaves the value to `carry`,
        which becomes this process's environment for the `docker` CLI itself --
        so the value never enters argv, and therefore never enters `ps`, which
        every user on the machine can read and which no log redaction covers.

        A declared secret in `env` is refused rather than redacted. Redaction
        would keep the run log clean and leave the value in the process
        listing, which is the leak that mattered: the Tauri key and its
        password reached `ps` this way, and reintroducing it is one keyword.
        """
        leaked = sorted(set(env or {}) & secret_env)
        if leaked:
            raise GateError(
                f"{', '.join(leaked)} would be written into argv as NAME=value, "
                "where `ps` can read it. Name it in `forward` and pass its value "
                "in `carry`, so docker takes it from its own environment."
            )
        network = require_container_network(network)
        argv = ["docker", "create", "--name", name, "--network", network]
        for key, value in (env or {}).items():
            argv += ["-e", f"{key}={value}"]
        argv += [part for name_only in forward for part in ("-e", name_only)]
        argv += [part for mount in mounts for part in ("-v", str(mount))]
        # Anonymous volumes: container-local writable space grafted over a
        # read-only tree. This is how a container that must write into its
        # source keeps those writes off the host, which is the difference
        # between "the container wrote" and "two processes shared an inode".
        argv += [part for path in scratch for part in ("-v", path)]
        if workdir is not None:
            argv += ["-w", workdir]
        argv += [image, *command]
        # Only `carry` reaches this process's environment. `env` is already in
        # argv, and setting it here as well would render every value twice and
        # blur the one distinction this signature exists to make.
        self._runner.run(argv, env=carry, secret_env=secret_env)

    def start(self, container: str, *, console: ConsoleMode = ConsoleMode.STREAM) -> None:
        self._runner.run(["docker", "start", "-a", container], console=console)

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
