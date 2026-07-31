from __future__ import annotations

import importlib.util
import os
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "release_test_binary.py"
SPEC = importlib.util.spec_from_file_location("release_test_binary", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
HELPER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HELPER)


class FakeRunner:
    def __init__(self, binary: Path) -> None:
        self.binary = binary
        self.commands: list[tuple[str, ...]] = []

    def __call__(
        self,
        command: tuple[str, ...],
        *,
        cwd: Path,
        check: bool,
        timeout: int,
    ) -> subprocess.CompletedProcess[str]:
        self.commands.append(command)
        assert cwd == ROOT
        assert check is True
        assert timeout == 120
        self.binary.parent.mkdir(parents=True, exist_ok=True)
        self.binary.write_bytes(b"locally built")
        self.binary.chmod(0o755)
        return subprocess.CompletedProcess(command, 0)


def test_release_mode_rejects_binary_missing_from_staged_package(
    tmp_path: Path,
) -> None:
    binary = tmp_path / "target/debug/capsem-gateway"

    with pytest.raises(FileNotFoundError, match="manifest-selected package"):
        HELPER.ensure_host_test_binary(
            binary,
            source_paths=(),
            build_command=("cargo", "build", "-p", "capsem-gateway"),
            project_root=ROOT,
            env={"CAPSEM_RELEASE_INPUT_DIR": str(tmp_path / "release-inputs")},
            runner=lambda *_args, **_kwargs: pytest.fail("release mode invoked cargo"),
        )


def test_release_mode_uses_exact_package_binary_regardless_of_checkout_mtime(
    tmp_path: Path,
) -> None:
    binary = tmp_path / "target/debug/capsem-gateway"
    source = tmp_path / "crates/capsem-gateway/src/main.rs"
    binary.parent.mkdir(parents=True)
    source.parent.mkdir(parents=True)
    binary.write_bytes(b"manifest package bytes")
    binary.chmod(0o755)
    source.write_text("newer checkout source", encoding="utf-8")
    os.utime(binary, (1, 1))
    os.utime(source, (2, 2))

    HELPER.ensure_host_test_binary(
        binary,
        source_paths=(source,),
        build_command=("cargo", "build", "-p", "capsem-gateway"),
        project_root=ROOT,
        env={"CAPSEM_RELEASE_INPUT_DIR": str(tmp_path / "release-inputs")},
        runner=lambda *_args, **_kwargs: pytest.fail("release mode invoked cargo"),
    )

    assert binary.read_bytes() == b"manifest package bytes"


def test_local_mode_builds_a_missing_binary(tmp_path: Path) -> None:
    binary = tmp_path / "target/debug/capsem-gateway"
    runner = FakeRunner(binary)

    HELPER.ensure_host_test_binary(
        binary,
        source_paths=(),
        build_command=("cargo", "build", "-p", "capsem-gateway"),
        project_root=ROOT,
        env={},
        runner=runner,
    )

    assert runner.commands == [("cargo", "build", "-p", "capsem-gateway")]
    assert binary.read_bytes() == b"locally built"


def test_local_mode_rebuilds_only_when_source_is_newer(tmp_path: Path) -> None:
    binary = tmp_path / "target/debug/capsem-admin"
    source = tmp_path / "crates/capsem-admin/src/main.rs"
    binary.parent.mkdir(parents=True)
    source.parent.mkdir(parents=True)
    binary.write_bytes(b"old")
    binary.chmod(0o755)
    source.write_text("new", encoding="utf-8")
    os.utime(binary, (1, 1))
    os.utime(source, (2, 2))
    runner = FakeRunner(binary)

    HELPER.ensure_host_test_binary(
        binary,
        source_paths=(source,),
        build_command=("cargo", "build", "-p", "capsem-admin"),
        project_root=ROOT,
        env={},
        runner=runner,
    )

    assert runner.commands == [("cargo", "build", "-p", "capsem-admin")]
