"""Pinned Cargo path dependencies must survive every cold helper build."""

from pathlib import Path

import pytest
from capsem_builder.gate import config as gate_config
from capsem_builder.gate.shellsurfaces import invocations, run_instructions
from capsem_builder.image.config import load_guest_config

ROOT = Path(__file__).resolve().parents[2]
CONFIG = gate_config.load(ROOT)
BUILD = load_guest_config(ROOT / "config/docker/image").build
HELPERS = (
    (BUILD.guest_rust_builder, "/prefetch", BUILD.guest_rust_builder.dockerfile),
    (CONFIG.package.builder, "/prefetch", CONFIG.package.builder.dockerfile),
    (CONFIG.install.builder, "/prefetch", CONFIG.install.builder.dockerfile),
    (CONFIG.hostimage, "/src", CONFIG.hostimage.base_dockerfile),
)
VENDOR_RATIONALE = (
    "Cargo loads path patches before fetching any registry package. Missing "
    "vendored sources fail only when a cached helper finally rebuilds; their "
    "manifests must also key that helper's dependency resolution."
)


def _fetches(source: str) -> bool:
    return any(
        argv[1:2] == ("fetch",)
        for body in run_instructions(source)
        for argv in invocations(body, "cargo")
    )


def _copied_before_fetch(source: str, copy: str) -> bool:
    lines = source.splitlines()
    if copy not in lines or not _fetches(source):
        return False
    return not _fetches("\n".join(lines[:lines.index(copy)]))


@pytest.mark.parametrize(("_settings", "destination", "dockerfile"), HELPERS)
def test_cold_helpers_copy_vendor_sources_before_fetch(_settings, destination, dockerfile):
    source = (ROOT / dockerfile).read_text()
    copy = f"COPY third_party/rust {destination}/third_party/rust"
    assert _copied_before_fetch(source, copy), VENDOR_RATIONALE


@pytest.mark.parametrize(("settings", "_destination", "_dockerfile"), HELPERS)
def test_vendor_manifests_are_part_of_every_helper_identity(settings, _destination, _dockerfile):
    manifests = set((ROOT / "third_party/rust").glob("*/Cargo.toml"))
    assert manifests, "vendored dependency inventory must not be empty"
    inputs = {ROOT / name for name in settings.identity_inputs}
    for pattern in getattr(settings, "identity_globs", ()):
        inputs.update(ROOT.glob(pattern))
    assert manifests <= inputs, VENDOR_RATIONALE + f" Missing: {manifests - inputs}"


@pytest.mark.parametrize(("_settings", "destination", "dockerfile"), HELPERS)
def test_commented_or_late_copies_do_not_hide_missing_dependencies(_settings, destination, dockerfile):
    source = (ROOT / dockerfile).read_text()
    copy = f"COPY third_party/rust {destination}/third_party/rust"
    assert _copied_before_fetch(source, copy), VENDOR_RATIONALE
    removed = source.replace(copy, "# " + copy)
    assert not _copied_before_fetch(removed, copy)
    assert not _copied_before_fetch(removed + "\n" + copy + "\n", copy)
