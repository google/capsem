"""Bounded storage write probes for package-manager and workspace paths."""

import pathlib

import pytest

from conftest import run


@pytest.mark.parametrize(
    "path",
    ["/usr/local", "/var/cache/apt", "/tmp", "/var/tmp", "/root"],
)
def test_bounded_write_probe(path):
    """Doctor must prove key writable paths can create, read, and delete."""
    target_dir = pathlib.Path(path)
    assert target_dir.is_dir(), f"{path} does not exist"

    probe = target_dir / ".capsem_write_probe"
    result = run(
        f'printf "capsem-storage-ok" > {probe} && '
        f"cat {probe} && "
        f"rm -f {probe}"
    )
    assert result.returncode == 0, (
        f"bounded write probe failed for {path}: "
        f"stdout={result.stdout!r} stderr={result.stderr!r}"
    )
    assert "capsem-storage-ok" in result.stdout
    assert not probe.exists(), f"write probe was not removed: {probe}"


def test_apt_partial_cache_writable_by_apt_user():
    """apt must be able to use its sandboxed _apt download cache."""
    partial = "/var/cache/apt/archives/partial"
    result = run(f"test -d {partial}")
    assert result.returncode == 0, f"{partial} is missing"

    result = run(
        "su -s /bin/sh _apt -c "
        f"'touch {partial}/.capsem_apt_probe && rm -f {partial}/.capsem_apt_probe'"
    )
    assert result.returncode == 0, (
        "_apt cannot write the apt partial cache; apt downloads will fall back "
        f"to unsandboxed root mode. stdout={result.stdout!r} stderr={result.stderr!r}"
    )
