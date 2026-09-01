"""VM components reuse exact input-keyed object receipts."""

from pathlib import Path

from capsem_builder.image.componentcache import (
    build_identity,
    input_digest,
    restore,
    source_digest,
    store,
)

ROOT = Path(__file__).resolve().parents[3]


def repository(tmp_path: Path) -> Path:
    config = tmp_path / "config"
    config.mkdir()
    policy = (ROOT / "config/cache.toml").read_text(encoding="utf-8")
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


def test_changed_component_input_is_a_clean_miss(tmp_path: Path) -> None:
    repo = repository(tmp_path)

    assert restore(repo, "rootfs", input_digest({"source": "new"}), tmp_path / "out") is None


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
