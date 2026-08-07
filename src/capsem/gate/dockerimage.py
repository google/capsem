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

from .errors import GateError
from .proc import Runner


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
        no_cache: bool = False,
    ) -> None:
        """Build an image. The context streams from the CLI, so it does not
        have to be visible inside the Lima VM the way a bind mount does."""
        argv = ["docker", "build", "-t", tag, "-f", dockerfile]
        if no_cache:
            argv.append("--no-cache")
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
