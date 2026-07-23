from __future__ import annotations

import importlib.util
import re
from pathlib import Path

import pytest


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "check_public_surface.py"


def _load_checker():
    spec = importlib.util.spec_from_file_location("check_public_surface", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_public_surfaces_match_the_approved_exact_allowlists() -> None:
    _load_checker().check_policy()


def test_surface_extractors_do_not_silently_return_empty_sets() -> None:
    checker = _load_checker()
    surfaces = checker.current_surfaces()

    assert set(surfaces) == {"just", "capsem_cli", "http"}
    assert all(values for values in surfaces.values())
    assert all(values == sorted(set(values)) for values in surfaces.values())


def test_declared_count_drift_fails_closed(tmp_path: Path) -> None:
    checker = _load_checker()
    policy = (ROOT / "config" / "public-surface.toml").read_text()
    broken = tmp_path / "public-surface.toml"
    broken.write_text(policy.replace("[just]\ncount = 11", "[just]\ncount = 12"))

    with pytest.raises(checker.SurfaceError, match="policy count=12"):
        checker.check_policy(broken)


def test_rejects_unapproved_allowlist_entry(tmp_path: Path) -> None:
    checker = _load_checker()
    policy = (ROOT / "config" / "public-surface.toml").read_text()
    broken = tmp_path / "public-surface.toml"
    broken.write_text(
        policy.replace(
            '  "build",',
            '  "build",\n  "unapproved-command",',
            1,
        ).replace("[just]\ncount = 11", "[just]\ncount = 12")
    )

    with pytest.raises(checker.SurfaceError, match="missing=.*unapproved-command"):
        checker.check_policy(broken)


def test_project_skills_do_not_teach_retired_public_just_commands() -> None:
    retired = {
        "audit",
        "bench",
        "benchmark",
        "benchmark-compare",
        "build-assets",
        "build-host-image",
        "build-kernel",
        "build-rootfs",
        "build-ui",
        "clean",
        "coverage",
        "cross-compile",
        "cut-release",
        "dev-frontend",
        "dev-tui",
        "docs",
        "inspect-session",
        "install",
        "list-sessions",
        "prepare-release",
        "qualify-release",
        "query-session",
        "release",
        "run-ui",
        "sandbox-logs",
        "test-artifacts",
        "test-assets",
        "test-frontend",
        "test-gateway",
        "test-gateway-e2e",
        "test-host-package-sbom",
        "test-install",
        "test-linux-rust",
        "ui",
        "update-deps",
        "update-fixture",
        "update-prices",
    }
    command = re.compile(r"\bjust\s+([a-z][a-z0-9-]*)\b")
    failures: list[str] = []
    for path in sorted((ROOT / "skills").rglob("*.md")):
        for line_number, line in enumerate(path.read_text().splitlines(), start=1):
            for match in command.finditer(line):
                if match.group(1) in retired:
                    failures.append(
                        f"{path.relative_to(ROOT)}:{line_number}: {line.strip()}"
                    )

    assert not failures, (
        "project skills teach retired public Just commands; use the approved "
        "surface or the explicitly private owning primitive:\n" + "\n".join(failures)
    )
