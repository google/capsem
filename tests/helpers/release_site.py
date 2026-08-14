"""Serialized release-site builds with per-caller output snapshots.

Astro cannot isolate two concurrent builds of the same project. It stages
prerendered chunks in `<outDir>/.prerender/` when outDir is inside the project
root and falls back to `<root>/.astro/` when it is not -- see
`getOutDirWithinCwd` in `astro/dist/core/build/common.js`. Both are fixed paths
under `release-site/`, so overlapping builds delete each other's staging
mid-prerender ("Cannot find module .../.prerender/chunks/...") and `--outDir`
buys no isolation. The build itself has to be serialized.

Serializing the build is necessary but not sufficient. Callers used to render
into the shared `release-site/dist` and then read pages back out of it after
dropping the lock, so a build started by another worker could swap the pages
between one test's build and its assertions -- the same suite failing on a
different assertion each run. Every build here therefore copies its output into
a private per-process directory while the lock is still held, and callers read
only that snapshot. Nothing outside this module reads `release-site/dist`.

Snapshots are keyed by graph content, so the many gates that render the
checked-in fixture graph share a single build per process, while a mutated
graph always gets its own build in its own directory.
"""

from __future__ import annotations

import atexit
import fcntl
import hashlib
import os
import shutil
import subprocess
import tempfile
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
FIXTURE_GRAPH = (
    PROJECT_ROOT
    / "tests"
    / "capsem-release"
    / "fixtures"
    / "release-graph-stable-nightly.json"
)

# Astro's own output directory, shared by every release-site build in the repo.
# Only ever touched while the build lock is held.
_ASTRO_DIST = PROJECT_ROOT / "release-site" / "dist"

# Keyed to the directory it protects, not to the caller's environment.
#
# This was `$TMPDIR/capsem-release-site-build.lock`, which serializes two
# callers only if they agree about `TMPDIR` -- and they do not. The gate exports
# its own, a developer may export one, CI leaves it unset. Each got a private
# lock file, took it uncontended, and rendered into the one shared
# `release-site/dist` alongside the others. That surfaces as an Astro build
# exiting non-zero with its staging removed underneath it, intermittently,
# which reads as flake rather than as the collision it is.
#
# The resource is `_ASTRO_DIST`, so the lock is derived from the same
# repository every caller is building in.
_LOCK_PATH = PROJECT_ROOT / "target" / "capsem-release-site-build.lock"

_SNAPSHOT_ROOT = Path(tempfile.mkdtemp(prefix="capsem-release-site-"))
atexit.register(shutil.rmtree, _SNAPSHOT_ROOT, True)

_BUILT: set[Path] = set()


@contextmanager
def release_site_build_lock() -> Iterator[None]:
    """Serialize a release-site Astro build against every other one on the host.

    For callers that run `build:channel` themselves: that script renders through
    the shared `release-site/dist` before overlaying into its own output
    directory, so it needs the lock even though its result is already private.
    """
    _LOCK_PATH.parent.mkdir(parents=True, exist_ok=True)
    with _LOCK_PATH.open("w", encoding="utf-8") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        yield


def release_site_dist(graph_path: Path) -> Path:
    """Snapshot directory that a build of ``graph_path`` renders into."""
    digest = hashlib.sha256(graph_path.read_bytes()).hexdigest()[:16]
    return _SNAPSHOT_ROOT / digest


def build_release_site(graph_path: Path) -> Path:
    """Render ``graph_path`` and return a private copy of the generated pages."""
    dist = release_site_dist(graph_path)
    if dist in _BUILT:
        return dist
    with release_site_build_lock():
        # Astro leaves pages from a previous graph in place, so clear the shared
        # output first: the snapshot has to hold exactly what this graph renders.
        shutil.rmtree(_ASTRO_DIST, ignore_errors=True)
        result = subprocess.run(
            ["pnpm", "--dir", "release-site", "run", "build"],
            cwd=PROJECT_ROOT,
            env={
                **os.environ,
                "ASTRO_TELEMETRY_DISABLED": "1",
                # The *input* the loader reads. One name used to mean both
                # "graph to render from" and "directory to write into", so a
                # fixture build and an overlay could not be told apart.
                "CAPSEM_RELEASE_GRAPH": str(graph_path),
            },
            text=True,
            capture_output=True,
            check=False,
        )
        if result.returncode != 0:
            raise AssertionError(
                f"release-site build failed for {graph_path}\n"
                f"stdout:\n{result.stdout}\n"
                f"stderr:\n{result.stderr}"
            )
        shutil.rmtree(dist, ignore_errors=True)
        shutil.copytree(_ASTRO_DIST, dist)
    _BUILT.add(dist)
    return dist


def build_release_site_from_fixture() -> Path:
    """Render the checked-in stable/nightly fixture graph."""
    return build_release_site(FIXTURE_GRAPH)


# Snapshot the fixture graph renders into. A constant, not mutable module state:
# it is derived from the fixture's content the same way build_release_site
# derives its destination, so the two agree by construction.
RELEASE_SITE_DIST = release_site_dist(FIXTURE_GRAPH)
