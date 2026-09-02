"""VM components reuse exact input-keyed object receipts."""

import os
from pathlib import Path
from types import SimpleNamespace
from typing import cast
from unittest.mock import patch

import pytest
from capsem_builder.cache.config import load_policy
from capsem_builder.image.componentcache import (
    build_identity,
    input_digest,
    restore,
    source_digest,
    store,
)
from capsem_builder.image.componentcache import (
    current as component_current,
)
from capsem_builder.image.config import load_guest_config
from capsem_builder.image.guestbinarycache import current as guest_current
from capsem_builder.image.guestbinarycache import materialize
from capsem_builder.image.models import BuildConfig

ROOT = Path(__file__).resolve().parents[3]


def test_guest_binary_identity_names_only_its_output_inputs() -> None:
    roots = set(
        load_guest_config(ROOT / "config/docker/image").build.guest_rust_builder.source_roots
    )

    assert roots == {
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        "crates/capsem-agent",
        "crates/capsem-bench",
        "crates/capsem-foundation",
        "crates/capsem-proto",
        "build_system/builder/image/docker.py",
        "build_system/builder/image/guestbinarycache.py",
    }


def repository(tmp_path: Path) -> Path:
    config = tmp_path / "config"
    config.mkdir()
    policy = (ROOT / "config/cache.toml").read_text(encoding="utf-8").replace(
        'authority_environment = "CAPSEM_CACHE_AUTHORITY"',
        'authority_environment = "CAPSEM_TEST_CACHE_AUTHORITY"',
    )
    config.joinpath("cache.toml").write_text(policy, encoding="utf-8")
    return tmp_path


def test_component_receipt_restores_hardlinked_outputs(tmp_path: Path) -> None:
    repo = repository(tmp_path)
    output = repo / "cache/target/first"
    output.mkdir(parents=True)
    (output / "vmlinuz").write_bytes(b"kernel")
    (output / "initrd.img").write_bytes(b"initrd")
    identity = input_digest({"arch": "x86_64", "source": "one"})
    store(repo, "kernel", identity, output, ("vmlinuz", "initrd.img"))

    restored = repo / "cache/target/second"
    files = restore(repo, "kernel", identity, restored)

    assert files is not None
    assert (restored / "vmlinuz").read_bytes() == b"kernel"
    assert (restored / "initrd.img").read_bytes() == b"initrd"
    assert (restored / "vmlinuz").stat().st_ino == (output / "vmlinuz").stat().st_ino
    assert (restored / "initrd.img").stat().st_ino == (output / "initrd.img").stat().st_ino


def test_component_restores_through_a_shared_asset_lane_symlink(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    authority_root = tmp_path / "authority"
    prefix_root = tmp_path / "prefix"
    authority_root.mkdir()
    prefix_root.mkdir()
    authority = repository(authority_root)
    prefix = repository(prefix_root)
    monkeypatch.setenv(load_policy(prefix).authority_environment, str(authority))

    generation = authority / "cache/target/assets/generations/current"
    first = generation / "code/build-arm64/arm64"
    first.mkdir(parents=True)
    first.joinpath("vmlinuz").write_bytes(b"kernel")
    first.joinpath("initrd.img").write_bytes(b"initrd")
    first_link = prefix / "cache/target/tests/code/build-arm64"
    first_link.parent.mkdir(parents=True)
    first_link.symlink_to(first.parent, target_is_directory=True)
    identity = input_digest({"arch": "arm64", "source": "one"})
    store(prefix, "kernel", identity, first_link / "arm64", ("vmlinuz", "initrd.img"))

    second = generation / "co-work/build-arm64"
    second.mkdir(parents=True)
    second_link = prefix / "cache/target/tests/co-work/build-arm64"
    second_link.parent.mkdir(parents=True)
    second_link.symlink_to(second, target_is_directory=True)

    restored = restore(prefix, "kernel", identity, second_link / "arm64")

    assert restored is not None
    assert second.joinpath("arm64/vmlinuz").read_bytes() == b"kernel"
    assert second.joinpath("arm64/initrd.img").read_bytes() == b"initrd"


def test_changed_component_input_is_a_clean_miss(tmp_path: Path) -> None:
    repo = repository(tmp_path)

    assert restore(repo, "rootfs", input_digest({"source": "new"}), tmp_path / "out") is None


def test_component_receipt_recognizes_only_exact_current_outputs(tmp_path: Path) -> None:
    repo = repository(tmp_path)
    output = repo / "cache/target/current"
    output.mkdir(parents=True)
    payload = output / "agent"
    payload.write_bytes(b"current")
    identity = input_digest({"source": "one"})
    store(repo, "guest-binaries", identity, output, (payload.name,))

    assert component_current(repo, "guest-binaries", identity, output) == (payload,)

    replacement = output / ".replacement"
    replacement.write_bytes(b"changed")
    replacement.replace(payload)
    assert component_current(repo, "guest-binaries", identity, output) is None

    assert restore(repo, "guest-binaries", identity, output) == (payload,)
    payload.chmod(0o600)
    assert component_current(repo, "guest-binaries", identity, output) is None


def test_build_identity_ignores_commit_and_runtime_labels() -> None:
    stable = {
        "arch": "x86_64",
        "template": "kernel",
        "docker_platform": "linux/amd64",
        "dockerfile": {"digest": "one"},
        "build_context": {"hash": "two"},
        "dependency_image": {"id": "three"},
    }

    first = build_identity({**stable, "git_revision": "old", "runtime": "docker"})
    second = build_identity({**stable, "git_revision": "new", "runtime": "colima"})

    assert first == second


def test_source_digest_changes_with_source_bytes_and_names(tmp_path: Path) -> None:
    source = tmp_path / "crates/agent"
    source.mkdir(parents=True)
    rust = source / "main.rs"
    rust.write_text("fn old() {}", encoding="utf-8")
    first = source_digest(tmp_path, ("crates",))

    rust.write_text("fn new() {}", encoding="utf-8")
    second = source_digest(tmp_path, ("crates",))
    rust.rename(source / "lib.rs")
    third = source_digest(tmp_path, ("crates",))

    assert len({first, second, third}) == 3


def test_source_digest_ignores_checkout_mtime_refresh(tmp_path: Path) -> None:
    source = tmp_path / "crates/agent/main.rs"
    source.parent.mkdir(parents=True)
    source.write_text("fn main() {}", encoding="utf-8")
    before = source_digest(tmp_path, ("crates/agent",))

    os.utime(source, (4_000_000_000, 4_000_000_000))

    assert source_digest(tmp_path, ("crates/agent",)) == before


def test_source_digest_ignores_undeclared_crates(tmp_path: Path) -> None:
    agent = tmp_path / "crates/agent"
    agent.mkdir(parents=True)
    agent.joinpath("main.rs").write_text("fn main() {}", encoding="utf-8")
    first = source_digest(tmp_path, ("crates/agent",))

    admin = tmp_path / "crates/admin"
    admin.mkdir()
    admin.joinpath("main.rs").write_text("fn unrelated() {}", encoding="utf-8")

    assert source_digest(tmp_path, ("crates/agent",)) == first


def test_guest_binary_generation_is_compiled_once_across_repository_prefixes(
    tmp_path: Path,
) -> None:
    repo = repository(tmp_path)
    source = repo / "guest-input"
    source.write_text("source", encoding="utf-8")
    prefix_root = tmp_path / "prefix"
    prefix_root.mkdir()
    prefix = repository(prefix_root)
    prefix.joinpath("guest-input").write_text("source", encoding="utf-8")
    prefix.joinpath("cache").mkdir()
    prefix.joinpath("cache/objects").symlink_to(repo / "cache/objects", target_is_directory=True)
    build = cast(
        BuildConfig,
        SimpleNamespace(guest_rust_builder=SimpleNamespace(source_roots=("guest-input",))),
    )
    names = ("capsem-agent", "capsem-bench")
    calls = 0

    def compile_binaries(_build, _arch, _repo, output):
        nonlocal calls
        calls += 1
        output.mkdir(parents=True, exist_ok=True)
        binaries = tuple(output / name for name in names)
        for binary in binaries:
            binary.write_bytes(binary.name.encode())
            binary.chmod(0o555)
        return list(binaries)

    with patch(
        "capsem_builder.image.guestbinarycache.guestbuilder.image_tag",
        return_value="sealed-builder:one",
    ):
        first = materialize(
            build, "x86_64", repo, repo / "cache/target/rootfs", names, compile_binaries
        )
        second = materialize(
            build,
            "x86_64",
            prefix,
            prefix / "cache/target/initrd",
            names,
            compile_binaries,
        )
        os.utime(prefix / "guest-input", (4_000_000_000, 4_000_000_000))
        assert guest_current(
            build,
            "x86_64",
            prefix,
            prefix / "cache/target/initrd",
            names,
        )

    assert calls == 1
    assert [path.read_bytes() for path in first] == [path.read_bytes() for path in second]
    assert all(path.stat().st_mode & 0o777 == 0o555 for path in second)
