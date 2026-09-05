"""Older snapshots must never inherit another worktree's compiled source."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import tomllib
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[3]


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
    for key in (
        "CARGO", "RUSTC_WRAPPER", "CARGO_BUILD_RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
    ):
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
            ["cargo", "build", "--offline", "--message-format=json"],
            cwd=root, env=env, capture_output=True, text=True, timeout=30,
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
