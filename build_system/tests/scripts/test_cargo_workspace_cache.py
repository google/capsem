"""Older snapshots must never inherit another worktree's compiled source."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

import pytest
from capsem_builder.gate import boundedlease
from helpers.bounded import bounded

ROOT = Path(__file__).resolve().parents[3]


def test_compiler_wrapper_preserves_cargo_binary_paths_arguments_and_exit_status() -> None:
    wrapper = ROOT / "build_system/scripts/build/rustc-workspace-wrapper.sh"
    key = "CARGO_BIN_EXE_capsem-router"
    path = "/a path/with spaces/capsem-router"
    arguments = ["argument with spaces", "literal $HOME", "line\nbreak"]
    result = subprocess.run(
        [str(wrapper), sys.executable, "-c",
         "import json,os,sys; print(json.dumps([os.environ.get(sys.argv[1]), sys.argv[2:]])); sys.exit(23)",
         key, *arguments],
        env=dict(os.environ, **{key: path}), capture_output=True, text=True, timeout=5,
    )
    assert result.returncode == 23, result.stderr
    assert json.loads(result.stdout) == [path, arguments]


@pytest.mark.parametrize("outer_cache", [False, True])
def test_shared_target_isolates_workspace_source_and_keeps_dependencies_warm(
    tmp_path: Path, outer_cache: bool,
) -> None:
    config_text = (ROOT / ".cargo/config.toml").read_text()
    config = tomllib.loads(config_text)
    wrapper = config["build"].get("rustc-workspace-wrapper")
    dependency = tmp_path / "dependency"
    dependency.mkdir()
    (dependency / "Cargo.toml").write_text(
        '[package]\nname="external-dependency"\nversion="0.1.0"\n'
        '[lib]\npath="lib.rs"\n'
    )
    (dependency / "lib.rs").write_text('pub fn suffix() -> &\'static str { "warm" }\n')
    roots = []
    for name in ("older", "newer"):
        root = tmp_path / name
        (root / "src").mkdir(parents=True)
        (root / ".cargo").mkdir()
        (root / "Cargo.toml").write_text(
            '[package]\nname="cache-repro"\nversion="0.1.0"\nedition="2021"\n'
            '[dependencies]\nexternal-dependency={path="../dependency"}\n'
        )
        (root / ".cargo/config.toml").write_text(config_text)
        if wrapper:
            destination = root / wrapper
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / wrapper, destination)
        (root / "src/lib.rs").write_text(f'pub fn value() -> &\'static str {{ "{name}" }}\n')
        (root / "src/main.rs").write_text(
            'fn main() { println!("{} {}", cache_repro::value(), external_dependency::suffix()); }\n'
        )
        for source in root.rglob("*"):
            if source.is_file():
                os.utime(source, (1_000_000_000, 1_000_000_000))
        roots.append(root)
    target = tmp_path / "cache/target/cargo"
    env = dict(os.environ, CARGO_TARGET_DIR=str(target))
    # Exercise Cargo freshness itself, independent of any ambient compiler cache.
    unwrapped = (
        "CARGO", "RUSTC_WRAPPER", "CARGO_BUILD_RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
    )
    for key in unwrapped:
        env.pop(key, None)
    if outer_cache:
        # sccache's nested-wrapper detection requires CARGO, including probes
        # made before the first compilation. Fail at that exact boundary.
        cache = tmp_path / "outer-cache.sh"
        cache.write_text('#!/bin/sh\n[ -n "$CARGO" ] || exit 99\nexec "$@"\n')
        cache.chmod(0o755)
        env["RUSTC_WRAPPER"] = str(cache)
    for index, root in enumerate((roots[1], roots[0], roots[1], roots[1])):
        build = subprocess.run(
            bounded(["cargo", "build", "--offline", "--message-format=json"], 30,
                    env={"CARGO_TARGET_DIR": str(target),
                         "RUSTC_WRAPPER": env.get("RUSTC_WRAPPER", "")}),
            cwd=root, env=env, capture_output=True, text=True,
        )
        assert build.returncode == 0, build.stderr
        messages = [json.loads(line) for line in build.stdout.splitlines()]
        artifacts = [message for message in messages if message["reason"] == "compiler-artifact"]
        if index:
            external = [item for item in artifacts if item["target"]["name"] == "external_dependency"]
            assert external and all(item["fresh"] for item in external), artifacts
        if index >= 2:
            assert all(item["fresh"] for item in artifacts), artifacts
        result = subprocess.run(
            [str(target / "debug/cache-repro")],
            capture_output=True, text=True, timeout=5,
        )
        assert result.returncode == 0, result.stderr
        assert result.stdout.strip() == f"{root.name} warm", build.stderr
    if wrapper:
        # Each checkout's workspace units name their checkout, so retention can
        # keep the working set and reclaim a removed checkout's copies; the
        # shared dependency names none.
        fingerprints = target / "debug/.fingerprint"
        owners = {
            Path(os.readlink(path / "capsem-owner"))
            for path in fingerprints.glob("cache-repro-*")
            if (path / "capsem-owner").is_symlink()
        }
        assert owners == {root.resolve() for root in roots}
        assert not list(fingerprints.glob("external-dependency-*/capsem-owner"))


def _recorder(directory: Path, name: str) -> Path:
    """An executable called `name` that prints its own name and arguments."""
    directory.mkdir(parents=True, exist_ok=True)
    program = directory / name
    program.write_text(
        f'#!/bin/sh\nprintf "%s\\n" {name} "$@"\n'
    )
    program.chmod(0o755)
    return program


@pytest.mark.parametrize(
    ("wrapper", "compiler", "cached"),
    [
        ("sccache", "rustc", True),
        ("sccache", "clippy-driver", False),
        ("not-sccache", "rustc", False),
        (None, "rustc", False),
    ],
)
def test_workspace_wrapper_hands_a_rustc_unit_to_the_compiler_cache(
    tmp_path: Path, wrapper: str | None, compiler: str, cached: bool,
) -> None:
    """Issue #277: sccache took the wrapper for the compiler and the absolute
    rustc path for a second input, so every workspace unit was non-cacheable."""
    script = ROOT / "build_system/scripts/build/rustc-workspace-wrapper.sh"
    real = _recorder(tmp_path / "toolchain/bin", compiler)
    env = {key: value for key, value in os.environ.items() if key != "RUSTC_WRAPPER"}
    if wrapper is not None:
        env["RUSTC_WRAPPER"] = str(_recorder(tmp_path / "cache", wrapper))
    result = subprocess.run(
        [str(script), str(real), "--crate-name", "unit", "src/lib.rs"],
        env=env, capture_output=True, text=True, timeout=5, check=False,
    )
    assert result.returncode == 0, result.stderr
    called = result.stdout.splitlines()
    expected = [str(real), "--crate-name", "unit", "src/lib.rs"]
    assert called == (["sccache", *expected] if cached else [compiler, *expected[1:]])


def test_a_workspace_unit_is_served_from_sccache_on_a_second_build(tmp_path: Path) -> None:
    """The real pipeline Cargo runs, `sccache <wrapper> /abs/rustc ...`, twice
    into an empty output directory: the second compile is a cache hit."""
    sccache = shutil.which("sccache")
    rustc = subprocess.run(
        ["rustc", "--print", "sysroot"], capture_output=True, text=True, timeout=30, check=False,
    )
    if sccache is None or rustc.returncode:
        pytest.skip("needs sccache and rustc")
    compiler = Path(rustc.stdout.strip()) / "bin/rustc"
    short = Path(tempfile.mkdtemp(prefix="sccache-", dir="/tmp"))  # a socket path is short
    with boundedlease.leased(["cargo", "build"], ROOT, os.environ):
        _check_native_cache_hit(tmp_path, short, sccache, compiler)


def _check_native_cache_hit(tmp_path: Path, short: Path, sccache: str, compiler: Path) -> None:
    try:
        env = {
            **os.environ, "CARGO": "cargo", "RUSTC_WRAPPER": sccache,
            "SCCACHE_DIR": str(short / "cache"), "SCCACHE_SERVER_UDS": str(short / "s.sock"),
            "SCCACHE_CLIENT_SIDE": "1", "SCCACHE_IDLE_TIMEOUT": "0",
            "SCCACHE_BASEDIRS": str(tmp_path), "CARGO_INCREMENTAL": "0",
        }
        overrides = {key: env[key] for key in (
            "CARGO", "RUSTC_WRAPPER", "SCCACHE_DIR", "SCCACHE_SERVER_UDS", "SCCACHE_CLIENT_SIDE",
            "SCCACHE_IDLE_TIMEOUT", "SCCACHE_BASEDIRS", "CARGO_INCREMENTAL",
        )}
        (tmp_path / "src").mkdir()
        (tmp_path / "src/lib.rs").write_text("pub fn unit() -> u32 { 277 }\n")
        hits = []
        artifacts = []
        for _ in range(2):
            subprocess.run([sccache, "--zero-stats"], env=env, capture_output=True, timeout=30)
            out = tmp_path / "out"
            shutil.rmtree(out, ignore_errors=True)
            out.mkdir()
            build = subprocess.run(
                bounded([sccache, str(ROOT / "build_system/scripts/build/rustc-workspace-wrapper.sh"),
                 str(compiler), "--crate-name", "unit", "--edition=2021", "src/lib.rs",
                 "--crate-type", "lib", "--emit=dep-info,metadata,link",
                 "-C", "metadata=277", "-C", "extra-filename=-277", "--out-dir", str(out)], 120, env=overrides),
                cwd=tmp_path, env=env, capture_output=True, text=True, check=False,
            )
            assert build.returncode == 0, build.stderr
            assert (out / "libunit-277.rlib").is_file()
            artifacts.append(hashlib.sha256((out / "libunit-277.rlib").read_bytes()).hexdigest())
            stats = subprocess.run(
                [sccache, "--show-stats", "--stats-format=json"],
                env=env, capture_output=True, text=True, timeout=30, check=True,
            )
            hits.append(json.loads(stats.stdout)["stats"]["cache_hits"]["counts"].get("Rust", 0))
        assert hits == [0, 1]
        assert artifacts[0] == artifacts[1]
    finally:
        subprocess.run([sccache, "--stop-server"], env=env, capture_output=True, timeout=30)
        shutil.rmtree(short, ignore_errors=True)
