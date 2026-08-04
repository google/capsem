"""The Linux parity lane, holding its own bytes.

Native Linux exercises the `cfg(target_os = "linux")` branches directly; a Mac
host runs the same checked-in script in Docker, or Linux-only regressions stay
out of the local gate entirely.

What changed, and why it is not a refactor. The lane bind-mounted the live
checkout read-only, grafted two writable mounts back through it to retrieve
coverage, and inherited four named volumes that survive between runs. That
combination is what let a warm machine and a clean checkout disagree about one
commit, and the mount is what raced a host step churning hardlinks in the same
tree -- a release died here on an intermittent `Permission denied` reading a
file that was `0644` before and after.

Now: dependencies live in a base image keyed by the lockfiles that determine
them, the source is copied into a thin image on top, the container runs with
`--network none`, and the coverage comes back through `docker cp`. Nothing is
shared, so nothing can be raced, and nothing is inherited, so a cold machine
and a warm one run the same thing.
"""

from __future__ import annotations

import hashlib

from .actions import Action
from .config import GateConfig
from .context import Context
from .docker import Docker
from .errors import GateError
from .execution import Step, step
from .filesystem import make_dir
from .plan import Plan


def base_tag(config: GateConfig) -> str:
    """The base image's identity: its dependency inputs, hashed.

    Keyed by content rather than by channel or date, so a dependency change
    cannot reuse a stale image and an unchanged tree cannot be forced to
    rebuild one.
    """
    settings = config.hostimage
    digest = hashlib.blake2b(digest_size=8)
    for name in settings.lockfile_inputs:
        path = config.path(name)
        if not path.is_file():
            raise GateError(f"lockfile input {name} is missing, so the base image has no identity")
        digest.update(path.read_bytes())
    return settings.base_tag_template.format(digest=digest.hexdigest())


def require_base(config: GateConfig, docker: Docker) -> str:
    """Refuse to start rather than rebuild a multi-gigabyte image mid-gate.

    A `Cargo.lock` bump changes the tag. Building it here would turn a cached
    five-minute fetch into a forty-minute surprise at minute four, with
    network, inside a lane that is supposed to have none.
    """
    tag = base_tag(config)
    if not docker.image_exists(tag):
        raise GateError(
            f"no Linux parity base image for {tag}. Its dependencies changed; "
            f"run `just warm` to build it with network before the gate runs "
            "without."
        )
    return tag


class RunLane(Action, name="linux-rust-lane"):
    """Build the source into an image, run it sealed, copy the coverage out.

    One action rather than five steps because the container is a single
    resource with a lifetime: it must be removed on every path, and the copy
    must happen before the removal. Splitting that across steps would put the
    ordering in the graph, where a reshuffle can break it silently, instead of
    in a `finally` where it cannot.
    """

    def render(self) -> str:
        return "build, run and extract the Linux parity lane with no mounts and no network"

    def perform(self, context: Context) -> None:
        config = context.config
        settings = config.hostimage
        docker = Docker(context.runner)

        base = require_base(config, docker)
        docker.build(
            tag=settings.lane_tag,
            dockerfile=config.path(settings.lane_dockerfile).as_posix(),
            context=str(config.root),
            args=[f"BASE={base}"],
        )

        container = settings.lane_container
        destination = config.path(settings.extract_to)
        docker.remove(container)
        docker.create(
            name=container,
            image=settings.lane_tag,
            command=["bash", settings.script],
            network=settings.network,
            env={config.environment.linux_rust.output_dir: settings.container_output_dir},
        )
        try:
            docker.start(container)
        finally:
            # Before the removal, and on the failure path too: a lane that
            # fails is exactly when its coverage and nextest output are worth
            # having, and `--rm` would have destroyed both.
            make_dir(destination)
            # Contents, not the directory: `docker cp` nests otherwise, and the
            # coverage lands where nothing looks for it.
            docker.copy_out(container, settings.container_output_contents, str(destination))
            docker.remove(container)


def lane(plan: Plan, config: GateConfig, *, after: tuple[Step, ...] = ()) -> Step:
    """Compose the parity lane into a plan."""
    return plan.add(
        step("linux-rust", RunLane(), contends=(config.exclusive("docker_daemon"),)),
        after=after,
    )
