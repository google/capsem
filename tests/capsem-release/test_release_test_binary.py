from __future__ import annotations

import importlib.util
import json
import os
import runpy
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "build_system" / "scripts" / "release" / "release_test_binary.py"
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
    ) -> subprocess.CompletedProcess[str]:
        self.commands.append(command)
        assert cwd == ROOT
        assert check is True
        self.binary.parent.mkdir(parents=True, exist_ok=True)
        self.binary.write_bytes(b"locally built")
        self.binary.chmod(0o755)
        return subprocess.CompletedProcess(command, 0)


def test_custom_binary_cannot_be_attested_by_building_unrelated_cargo_output(
    tmp_path: Path,
) -> None:
    binary = tmp_path / "frozen" / "capsem-gateway"
    binary.parent.mkdir()
    binary.write_bytes(b"stale frozen bytes")
    binary.chmod(0o755)
    rebuilt = tmp_path / "cargo" / "debug" / binary.name
    runner = FakeRunner(rebuilt)
    with pytest.raises(RuntimeError, match="provenance"):
        HELPER.ensure_host_test_binary(
            binary,
            source_paths=(),
            build_command=("cargo", "build", "-p", "capsem-gateway"),
            project_root=ROOT,
            env={"CAPSEM_RELEASE_BIN_DIR": str(binary.parent)},
            runner=runner,
            build_output=runner.binary,
        )
    assert not HELPER._receipt_path(binary).exists()
    assert not runner.commands, "a frozen directory must not trigger a build elsewhere"


@pytest.mark.parametrize("schema", ["capsem.local-test-binary.v1", "capsem.local-test-binary.v2"])
def test_custom_binary_cannot_reuse_a_previously_misdirected_receipt(
    tmp_path: Path, schema: str
) -> None:
    binary = tmp_path / "frozen" / "capsem-gateway"
    binary.parent.mkdir()
    binary.write_bytes(b"unrelated frozen bytes")
    binary.chmod(0o755)
    command = ("cargo", "build", "-p", "capsem-gateway")
    env = {"CAPSEM_RELEASE_BIN_DIR": str(binary.parent)}
    receipt = HELPER._receipt_path(binary)
    receipt.write_text(
        json.dumps(
            {
                "schema": schema,
                "build_output": str(binary.resolve()),
                "source_digest": HELPER._source_digest(
                    (), project_root=ROOT, build_command=command, env=env
                ),
                "binary_digest": HELPER._digest_file(binary),
            }
        )
    )
    before = receipt.read_bytes()
    with pytest.raises(RuntimeError, match="provenance"):
        HELPER.ensure_host_test_binary(
            binary,
            source_paths=(),
            build_command=command,
            project_root=ROOT,
            env=env,
            runner=lambda *_args, **_kwargs: pytest.fail("custom receipt triggered build"),
        )
    assert receipt.read_bytes() == before


def test_release_mode_rejects_binary_missing_from_staged_package(
    tmp_path: Path,
) -> None:
    binary = tmp_path / "cache/target/cargo/debug/capsem-gateway"

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
    binary = tmp_path / "cache/target/cargo/debug/capsem-gateway"
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
    binary = tmp_path / "cache/target/cargo/debug/capsem-gateway"
    runner = FakeRunner(binary)

    HELPER.ensure_host_test_binary(
        binary,
        source_paths=(),
        build_command=("cargo", "build", "-p", "capsem-gateway"),
        project_root=ROOT,
        env={},
        runner=runner,
        build_output=runner.binary,
    )

    assert runner.commands == [
        (
            sys.executable,
            str(ROOT / "build_system/scripts/ci/run-bounded-command.py"),
            "--timeout-seconds",
            "120",
            "--",
            "cargo",
            "build",
            "-p",
            "capsem-gateway",
        )
    ]
    assert binary.read_bytes() == b"locally built"


def _frozen_cohort(tmp_path: Path) -> tuple[Path, Path, Path, dict[str, str]]:
    from capsem_builder.release.tools.test_binary_provenance import (
        COHORT_SCHEMA,
        COHORT_VARIABLE,
    )

    root = tmp_path / "source"
    root.mkdir()
    subprocess.run(("git", "init", "-q", str(root)), check=True, timeout=10)
    (root / "source.rs").write_text("fn main() {}\n")
    subprocess.run(("git", "-C", str(root), "add", "source.rs"), check=True, timeout=10)
    subprocess.run(
        (
            "git",
            "-C",
            str(root),
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.test",
            "commit",
            "--no-gpg-sign",
            "-qm",
            "fixture",
        ),
        check=True,
        timeout=10,
    )

    def git(ref: str) -> str:
        return subprocess.run(
            ("git", "-C", str(root), "rev-parse", ref),
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        ).stdout.strip()

    binary = tmp_path / "frozen" / "capsem-gateway"
    binary.parent.mkdir()
    binary.write_bytes(b"verified build owner bytes")
    binary.chmod(0o755)
    manifest = tmp_path / "cohort.json"
    manifest.write_text(
        json.dumps(
            {
                "schema": COHORT_SCHEMA,
                "source_commit": git("HEAD"),
                "source_tree": git("HEAD^{tree}"),
                "bin_dir": str(binary.parent),
                "binaries": {binary.name: HELPER._digest_file(binary)},
            }
        )
    )
    return (
        root,
        binary,
        manifest,
        {COHORT_VARIABLE: str(manifest), "CAPSEM_RELEASE_BIN_DIR": str(binary.parent)},
    )


def test_verified_frozen_cohort_is_read_only_and_never_rebuilt(tmp_path: Path) -> None:
    root, binary, manifest, env = _frozen_cohort(tmp_path)
    before = manifest.read_bytes(), binary.read_bytes()
    for _ in range(2):
        HELPER.ensure_host_test_binary(
            binary,
            source_paths=(),
            build_command=("cargo", "build", "-p", "capsem-gateway"),
            project_root=root,
            env=env,
            runner=lambda *_args, **_kwargs: pytest.fail("frozen cohort invoked build"),
        )
    assert before == (manifest.read_bytes(), binary.read_bytes())
    assert not HELPER._receipt_path(binary).exists()


@pytest.mark.parametrize(
    "mutation",
    ["binary", "source", "tree", "directory", "schema", "untracked", "permission", "missing"],
)
def test_frozen_cohort_requires_matching_source_and_artifact(tmp_path: Path, mutation: str) -> None:
    root, binary, manifest, env = _frozen_cohort(tmp_path)
    payload = json.loads(manifest.read_text())
    if mutation == "binary":
        binary.write_bytes(b"unrelated frozen bytes")
    elif mutation == "source":
        (root / "source.rs").write_text("fn main() { panic!(); }\n")
    elif mutation == "tree":
        payload["source_tree"] = "0" * 40
    elif mutation == "directory":
        payload["bin_dir"] = str(tmp_path / "foreign")
    elif mutation == "schema":
        payload.pop("schema")
    elif mutation == "permission":
        binary.chmod(0o644)
    elif mutation == "missing":
        binary.unlink()
    else:
        (root / "untracked.rs").write_text("untracked binary input\n")
    manifest.write_text(json.dumps(payload))
    with pytest.raises(RuntimeError, match="provenance"):
        HELPER.ensure_host_test_binary(
            binary,
            source_paths=(),
            build_command=("cargo", "build", "-p", "capsem-gateway"),
            project_root=root,
            env=env,
            runner=lambda *_args, **_kwargs: pytest.fail("invalid cohort invoked build"),
        )
    assert not HELPER._receipt_path(binary).exists()


def test_benchmark_selects_the_declared_frozen_binary_root(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    selected = tmp_path / "frozen"
    monkeypatch.setenv("CAPSEM_RELEASE_BIN_DIR", str(selected))
    namespace = runpy.run_path(str(ROOT / "tests/test_capsem_bench_rust.py"))
    assert namespace["BENCH_BINARY"] == selected / "capsem-bench-rs"


def test_local_binary_fallback_takes_the_machine_lease(tmp_path: Path) -> None:
    runner = FakeRunner(tmp_path / "capsem-gateway")
    HELPER.ensure_host_test_binary(
        runner.binary,
        source_paths=(),
        build_command=("cargo", "build", "-p", "capsem-gateway"),
        project_root=ROOT,
        env={},
        runner=runner,
        build_output=runner.binary,
    )
    assert runner.commands == [
        (
            sys.executable,
            str(ROOT / "build_system/scripts/ci/run-bounded-command.py"),
            "--timeout-seconds",
            "120",
            "--",
            "cargo",
            "build",
            "-p",
            "capsem-gateway",
        )
    ], "a local fixture fallback must not rebuild shared binaries outside the machine lease"


def test_local_mode_rebuilds_when_source_has_no_receipt(tmp_path: Path) -> None:
    binary = tmp_path / "cache/target/cargo/debug/capsem-admin"
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
        build_output=runner.binary,
    )

    assert runner.commands == [
        (
            sys.executable,
            str(ROOT / "build_system/scripts/ci/run-bounded-command.py"),
            "--timeout-seconds",
            "120",
            "--",
            "cargo",
            "build",
            "-p",
            "capsem-admin",
        )
    ]


def test_local_mode_rebuilds_a_newer_binary_from_another_worktree(
    tmp_path: Path,
) -> None:
    binary = tmp_path / "cache/target/cargo/debug/capsem-mock-server"
    source = tmp_path / "crates/capsem-mock-server/src/main.rs"
    binary.parent.mkdir(parents=True)
    source.parent.mkdir(parents=True)
    source.write_text("current source", encoding="utf-8")
    binary.write_bytes(b"newer foreign binary")
    binary.chmod(0o755)
    os.utime(source, (1, 1))
    os.utime(binary, (2, 2))
    runner = FakeRunner(binary)

    HELPER.ensure_host_test_binary(
        binary,
        source_paths=(source,),
        build_command=("cargo", "build", "-p", "capsem-mock-server"),
        project_root=ROOT,
        env={},
        runner=runner,
        build_output=runner.binary,
    )

    assert len(runner.commands) == 1
    assert binary.read_bytes() == b"locally built"


def test_local_mode_reuses_receipt_until_shared_binary_changes(tmp_path: Path) -> None:
    binary = tmp_path / "cache/target/cargo/debug/capsem-mock-server"
    source = tmp_path / "crates/capsem-mock-server/src/main.rs"
    source.parent.mkdir(parents=True)
    source.write_text("current source", encoding="utf-8")
    runner = FakeRunner(binary)

    def ensure() -> None:
        HELPER.ensure_host_test_binary(
            binary,
            source_paths=(source,),
            build_command=("cargo", "build", "-p", "capsem-mock-server"),
            project_root=ROOT,
            env={},
            runner=runner,
            build_output=runner.binary,
        )

    ensure()
    ensure()
    assert len(runner.commands) == 1

    binary.write_bytes(b"foreign worktree output")
    binary.chmod(0o755)
    ensure()

    assert len(runner.commands) == 2
    assert binary.read_bytes() == b"locally built"
