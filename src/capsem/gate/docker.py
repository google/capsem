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
    """A bind mount or named volume, in `-v` order."""

    source: str
    target: str
    options: str = ""

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
        options: list[str] | None = None,
        mounts: list[Mount] | None = None,
    ) -> None:
        argv = ["docker", "run", "-d", "--name", name, *(options or [])]
        for mount in mounts or []:
            argv += ["-v", str(mount)]
        argv += [image, *command]
        self._runner.run(argv)

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
