"""Release-site builds go through `helpers.release_site`, never by hand.

Astro stages prerendered chunks at a path fixed under the project root, so two
concurrent builds of `release-site` delete each other's staging mid-prerender
and no `--outDir` makes them independent. Every build on a host must therefore
serialize, and -- because the pages land in the one shared `release-site/dist`
-- every reader must take its own copy before the next build overwrites them.

Both rules had been rediscovered by hand seven times across thirteen files.
Four of those copies took a lock that covered the build but not the reads it
existed to protect, which is why the release-site gates failed on a different
assertion every run under `pytest -n`.

**Serialize.** A module that spawns a release-site build reaches the lock
through `helpers.release_site` -- `build_release_site` for a rendered graph,
`release_site_build_lock` for a `build:channel` that manages its own output.

**Snapshot.** `release-site/dist` is the shared staging area, not a place to
read results from. Only the helper may name it; callers read the private
directory a build returns.

A wrapper nobody is obliged to use is a suggestion. These make it the only way.
"""

from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent
TESTS_ROOT = PROJECT_ROOT / "tests"

# Owns both rules, so it is the one place allowed to implement them.
OWNER = TESTS_ROOT / "helpers" / "release_site.py"

# `["pnpm", ...]` as a subprocess argv. Deliberately not a bare "pnpm" match:
# several gates assert on the *text* of shell scripts that run pnpm, and
# asserting on a command is not spawning one.
PNPM_ARGV = re.compile(r'\[\s*"pnpm"')

# Builds `release-site` either by naming it in the argv or by running there.
RELEASE_SITE_BUILD = re.compile(
    r'"--dir",\s*"release-site"|cwd=\w+\s*/\s*"release-site"'
)

# The host `release-site/dist`, built as a path. Container-side literals such
# as "/src/release-site/dist" name a bind mount inside the install image and
# are a different thing entirely.
HOST_DIST = re.compile(r'"release-site"\s*(?:/|,)\s*"dist"')

REACHES_HELPER = re.compile(r"from helpers\.release_site import|helpers\.release_site")


def _test_sources() -> list[Path]:
    return sorted(path for path in TESTS_ROOT.rglob("*.py") if path != OWNER)


def test_release_site_builds_take_the_shared_lock() -> None:
    offenders = []
    for path in _test_sources():
        source = path.read_text(encoding="utf-8")
        if not (PNPM_ARGV.search(source) and RELEASE_SITE_BUILD.search(source)):
            continue
        if not REACHES_HELPER.search(source):
            offenders.append(path.relative_to(PROJECT_ROOT))

    assert not offenders, (
        "these modules spawn a release-site build without reaching the shared "
        "lock in helpers.release_site, so they race every other build on the "
        f"host: {[str(path) for path in offenders]}"
    )


def test_only_the_helper_names_the_shared_dist() -> None:
    offenders = [
        path.relative_to(PROJECT_ROOT)
        for path in _test_sources()
        if HOST_DIST.search(path.read_text(encoding="utf-8"))
    ]

    assert not offenders, (
        "release-site/dist is shared staging that the next build overwrites; "
        "read the private directory build_release_site returns instead of "
        f"naming it: {[str(path) for path in offenders]}"
    )
