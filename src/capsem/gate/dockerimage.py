"""What the gate does to images, as opposed to containers.

Split from `docker` at the module ceiling, and along the seam the boundary
guard already named: images are built, keyed and reclaimed on a different
rhythm from the containers that run them -- a base image outlives hundreds of
runs, a container outlives one step.

A mixin rather than a second class, so no call site has to know which half of
`Docker` it is reaching for. The split is about what a reader has to hold at
once, not about handing callers two objects to thread around.
"""

from __future__ import annotations

import json
import re

from .errors import GateError
from .invocation import ConsoleMode
from .proc import Runner

BUILDKIT_NETWORKS = frozenset({"default", "host", "none"})
CONTAINER_NETWORKS = frozenset({"bridge", "host", "none"})


def require_build_network(network: str) -> str:
    if network not in BUILDKIT_NETWORKS:
        allowed = ", ".join(sorted(BUILDKIT_NETWORKS))
        raise GateError(f"invalid BuildKit network {network!r}; expected one of: {allowed}")
    return network


def require_container_network(network: str) -> str:
    if network not in CONTAINER_NETWORKS:
        allowed = ", ".join(sorted(CONTAINER_NETWORKS))
        raise GateError(f"invalid container network {network!r}; expected one of: {allowed}")
    return network


class ImageOperations:
    """Build, identify and interrogate images. Mixed into `Docker`."""

    _runner: Runner
    """Provided by `Docker`; declared so this half type-checks on its own."""

    # -- images ------------------------------------------------------------

    def build(
        self,
        *,
        tag: str,
        dockerfile: str,
        context: str,
        args: list[str] | None = None,
        platform: str | None = None,
        network: str | None = None,
        console: ConsoleMode = ConsoleMode.STREAM,
        no_cache: bool = False,
    ) -> None:
        """Build an image. The context streams from the CLI, so it does not
        have to be visible inside the Lima VM the way a bind mount does."""
        argv = ["docker", "build", "-t", tag, "-f", dockerfile]
        if platform is not None:
            argv += ["--platform", platform]
        if network is not None:
            argv += ["--network", require_build_network(network)]
        if no_cache:
            argv.append("--no-cache")
        for value in args or []:
            argv += ["--build-arg", value]
        argv.append(context)
        self._runner.run(argv, console=console)

    def read(
        self,
        *,
        image: str,
        command: list[str],
        network: str,
        options: tuple[str, ...] = (),
        mounts: tuple[object, ...] = (),
        workdir: str | None = None,
        check: bool = True,
    ) -> str:
        """Run a container to completion and return what it printed.

        The third thing a call site can want from a container, after "do it"
        and "did it work": the answer itself. A probe asks an image
        to read the checkout's revision as a stranger, and without this it had
        to assemble its own `docker run` -- and pick its own network mode.
        """
        argv = [
            "docker",
            "run",
            "--rm",
            "--network",
            require_container_network(network),
            *options,
        ]
        argv += [part for mount in mounts for part in ("-v", str(mount))]
        if workdir is not None:
            argv += ["-w", workdir]
        argv += [image, *command]
        return self._runner.capture(argv, check=check)

    def image_exists(self, tag: str, *, platform: str | None = None) -> bool:
        argv = ["docker", "image", "inspect"]
        if platform is not None:
            argv += ["--platform", platform]
        return self._runner.succeeds([*argv, tag])

    def pull(self, image: str, *, platform: str) -> None:
        """Materialize one exact platform image through the Docker daemon."""
        self._runner.run(["docker", "pull", "--platform", platform, image])

    def image_id(self, tag: str, *, platform: str | None = None) -> str:
        """The exact image a mutable tag currently names.

        `:latest` is a pointer, so "the same tag" is a different image after
        every rebuild. Anything that keys a cache off a parent image has to key
        it off this, or it reuses work built against an image that no longer
        exists under that name.
        """
        argv = ["docker", "image", "inspect"]
        if platform is not None:
            argv += ["--platform", platform]
        found = self._runner.capture([*argv, "--format", "{{.Id}}", tag]).strip()
        if not found:
            raise GateError(f"docker has no image tagged {tag}, so nothing can be keyed by it")
        return found

    def image_reference(self, tag: str) -> str:
        """Return the immutable repository-qualified digest behind a local tag.

        A bare image ID works for `docker run` but not in Dockerfile `FROM`:
        BuildKit parses `sha256:<hex>` as a repository and tries to pull it.
        The local RepoDigest names the same exact index in valid FROM syntax.
        """
        raw = self._runner.capture(
            ["docker", "image", "inspect", "--format", "{{json .RepoDigests}}", tag]
        ).strip()
        try:
            candidates = json.loads(raw)
        except json.JSONDecodeError:
            candidates = None
        if not isinstance(candidates, list):
            candidates = []
        name = tag.split("@", 1)[0]
        if name.rfind(":") > name.rfind("/"):
            name = name.rsplit(":", 1)[0]
        matches = {
            candidate
            for candidate in candidates or []
            if isinstance(candidate, str)
            and candidate.startswith(f"{name}@")
            and re.fullmatch(r"[^@\s]+@sha256:[0-9a-f]{64}", candidate)
        }
        if len(matches) != 1:
            raise GateError(
                f"docker image {tag} has no unique matching repository digest; found {raw!r}"
            )
        return matches.pop()

    def image_label(self, tag: str, label: str) -> str:
        """Read the label used to reject accidental warm-tag poisoning."""
        return self._runner.capture(
            [
                "docker",
                "image",
                "inspect",
                "--format",
                f'{{{{ index .Config.Labels "{label}" }}}}',
                tag,
            ]
        ).strip()
