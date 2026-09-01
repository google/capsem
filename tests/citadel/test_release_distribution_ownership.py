"""Keep the release-distribution generator and its output under one owner."""

from __future__ import annotations

import re
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OWNER = ROOT / "build_system" / "release_site"
UNIT_TEST = ROOT / "build_system" / "tests" / "release_site" / "release-data.test.ts"
SHARED_DIST = Path("build_system", "release_site", "dist").as_posix()
EXPECTED = {
    ".gitignore",
    ".npmrc",
    "astro.config.mjs",
    "package.json",
    "pnpm-lock.yaml",
    "pnpm-workspace.yaml",
    "scripts/check-release-site-contract.py",
    "scripts/overlay-dist.mjs",
    "scripts/release_site_snapshot.py",
    "scripts/serve-release-test-root.py",
    "scripts/write-release-site-ci-fixture.py",
    "src/layouts/ReleaseLayout.astro",
    "src/lib/release-data.ts",
    "src/pages/404.astro",
    "src/pages/channels/[channel]/packages/[id].astro",
    "src/pages/channels/[channel]/profiles/[id].astro",
    "src/pages/channels/[id].astro",
    "src/pages/index.astro",
    "src/pages/profiles/[id].astro",
    "src/styles/global.css",
    "tsconfig.json",
    "vitest.config.ts",
}
LEGACY = re.compile(
    r"(?<![A-Za-z0-9_./-])release-site/|cache/target/release-channel(?=$|[/\'\"\s])"
)
LEGACY_LOCAL_OUTPUT = re.compile(
    r"cache/target/(?:install-test-channel|release-rehearsal/dist)(?=$|[/\'\"\s])"
)
RATIONALE = """\
The Astro release site is a build-time distribution generator, not an
independent product website. Its source and unit tests belong to build_system,
while cross-system deployment acceptance stays under root tests. Generated
distribution bytes have one repository output root, cache/target/distribution. Old
source or output literals can make local, CI, package, and deployment lanes
build different trees. See T3 and cache/target/ in the repository cleanup proposal.
"""


def _toml(path: Path) -> dict[str, object]:
    return tomllib.loads(path.read_text(encoding="utf-8"))


def _tracked_text() -> dict[str, str]:
    paths = subprocess.run(
        ("git", "ls-files"),
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.splitlines()
    sources: dict[str, str] = {}
    for relative in paths:
        if relative.startswith("tests/citadel/"):
            continue
        path = ROOT / relative
        if path.is_symlink() or not path.is_file():
            continue
        raw = path.read_bytes()
        if b"\0" in raw:
            continue
        try:
            sources[relative] = raw.decode("utf-8")
        except UnicodeDecodeError:
            continue
    return sources


def _legacy_literals(sources: dict[str, str]) -> list[str]:
    return sorted(path for path, text in sources.items() if LEGACY.search(text))


def _legacy_local_outputs(sources: dict[str, str]) -> list[str]:
    return sorted(
        path
        for path, text in sources.items()
        if LEGACY_LOCAL_OUTPUT.search(text)
    )


def _release_source_modes() -> dict[str, str]:
    modes: dict[str, str] = {}
    output = subprocess.run(
        ("git", "ls-files", "--stage", "build_system/release_site"),
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    for line in output.splitlines():
        mode, _object, _stage, path = line.split(maxsplit=3)
        relative = Path(path).relative_to("build_system/release_site").as_posix()
        modes[relative] = mode
    return modes


def test_legacy_literal_detector_observes_both_old_owners() -> None:
    found = _legacy_literals(
        {
            "source": "release-site/package.json",
            "output": "--out-dir cache/target/release-channel",
            "valid": "build_system/release_site and cache/target/distribution",
        }
    )
    assert found == ["output", "source"], RATIONALE


def test_local_output_detector_observes_pre_convergence_paths() -> None:
    found = _legacy_local_outputs(
        {
            "install": 'channel = "cache/target/install-test-channel"',
            "rehearsal": "cache/target/release-rehearsal/dist/assets/stable/manifest.json",
            "valid": "cache/target/distribution/rehearsal/dist/assets/stable/manifest.json",
            "crates/capsem/src/tests.rs": "cache/target/install-test-channel",
        }
    )
    assert found == ["crates/capsem/src/tests.rs", "install", "rehearsal"], RATIONALE


def test_release_generator_and_unit_test_have_exact_build_system_owners() -> None:
    modes = _release_source_modes()
    assert set(modes) == EXPECTED, RATIONALE
    assert not (ROOT / "release-site").exists(), RATIONALE
    assert UNIT_TEST.is_file(), RATIONALE
    for relative, mode in modes.items():
        assert mode == "100644", (
            f"{RATIONALE}\n{relative}: tracked mode is {mode}, expected 100644"
        )
        ignored = subprocess.run(
            (
                "git",
                "check-ignore",
                "--no-index",
                "--quiet",
                f"build_system/release_site/{relative}",
            ),
            cwd=ROOT,
            check=False,
        )
        assert ignored.returncode == 1, f"{RATIONALE}\n{relative}: source is ignored"


def test_config_owns_new_source_and_distribution_paths() -> None:
    config = _toml(ROOT / "config" / "gate.toml")
    outputs = config["outputs"]
    assert isinstance(outputs, dict)
    distribution = outputs["distribution"]
    install = config["install"]
    assert isinstance(install, dict)
    assert install["release_site_dir"] == "build_system/release_site"
    identity = install["builder"]["identity_inputs"]
    assert all(
        f"build_system/release_site/{name}" in identity
        for name in ("package.json", "pnpm-lock.yaml", "pnpm-workspace.yaml")
    ), RATIONALE
    assert install["layout"]["extra_owned_paths"] == [
        "build_system/release_site/node_modules",
        SHARED_DIST,
    ], RATIONALE
    assert install["layout"]["channel"] == f"{distribution}/install-proof", RATIONALE
    modules = config["modules"]
    assert isinstance(modules, dict)
    assert modules["rehearsal_work_dir"] == f"{distribution}/rehearsal", RATIONALE
    assert modules["rehearsal_after_manifest"] == (
        f"{distribution}/rehearsal/dist/assets/{{channel}}/manifest.json"
    ), RATIONALE
    disk = config["disk"]
    assert isinstance(disk, dict)
    reclaimable = disk["reclaimable"]
    assert "cache/target/distribution" in reclaimable, RATIONALE
    assert "cache/target/release-channel" not in reclaimable, RATIONALE


def test_no_old_release_source_or_distribution_output_literal_remains() -> None:
    sources = _tracked_text()
    assert not _legacy_literals(sources), RATIONALE
    assert not _legacy_local_outputs(sources), RATIONALE


def test_cross_system_release_acceptance_stays_at_repository_root() -> None:
    assert (ROOT / "tests/capsem-release/test_release_channel_contract.py").is_file()
    assert (ROOT / "tests/capsem-release/test_release_output_contract.py").is_file()
