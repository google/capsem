from pathlib import Path

from helpers.service import binary_dir_from_env


def test_binary_dir_defaults_to_target_debug(monkeypatch):
    monkeypatch.delenv("CAPSEM_TEST_BIN_DIR", raising=False)

    assert binary_dir_from_env(Path("/repo")) == Path("/repo/target/debug")


def test_binary_dir_can_use_release_binaries(monkeypatch):
    monkeypatch.setenv("CAPSEM_TEST_BIN_DIR", "target/release")

    assert binary_dir_from_env(Path("/repo")) == Path("/repo/target/release")


def test_binary_dir_accepts_absolute_override(monkeypatch, tmp_path):
    monkeypatch.setenv("CAPSEM_TEST_BIN_DIR", str(tmp_path))

    assert binary_dir_from_env(Path("/repo")) == tmp_path
