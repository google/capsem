"""The maintained dependency scanner is exact, ordered, and fail closed."""

from __future__ import annotations

import subprocess
from pathlib import Path

from capsem_builder.cache.contract import CacheScope, PruneStrategy
from capsem_builder.cache.models import CachePolicy, StagePolicy
from capsem_builder.cache.paths import CachePaths
from capsem_builder.cache.tools import MaterializedTool
from capsem_builder.gate.config import for_root
from capsem_builder.gate.tools.audit import dependencies as audit

PROJECT_ROOT = Path(__file__).resolve().parents[3]
POLICY = for_root(PROJECT_ROOT).audits.dependency_policy


def _paths(tmp_path: Path) -> CachePaths:
    return CachePaths(
        repository_root=tmp_path,
        policy=CachePolicy(
            version=1,
            root=Path("cache"),
            authority_environment="CAPSEM_TEST_CACHE_AUTHORITY",
            stages={
                "audit-results": StagePolicy(
                    path=Path("tools/audits"),
                    description="test audit verdicts",
                    scope=CacheScope.DISK,
                    warm_size_bytes=1,
                    max_size_bytes=2,
                    prune_strategy=PruneStrategy.LRU,
                    maximum_age_hours=1,
                )
            },
        ),
    )


def _tool(tmp_path: Path) -> MaterializedTool:
    executable = tmp_path / "osv-scanner"
    executable.write_bytes(b"scanner")
    return MaterializedTool(path=executable, sha256="0" * 64, cache_hit=True)


def test_clean_scan_covers_every_configured_lockfile_then_reuses_exact_verdict(
    monkeypatch, tmp_path: Path
) -> None:
    paths = _paths(tmp_path)
    monkeypatch.setattr(audit, "load_paths", lambda _root: paths)
    commands: list[list[str]] = []
    resolutions = 0

    def resolve(_paths, _policy):
        nonlocal resolutions
        resolutions += 1
        return _tool(tmp_path)

    def runner(command, **_kwargs):
        commands.append(command)
        return subprocess.CompletedProcess(command, 0, stdout="", stderr="")

    assert audit.audit_dependencies(PROJECT_ROOT, POLICY, runner=runner, resolve=resolve) == 0
    assert audit.audit_dependencies(PROJECT_ROOT, POLICY, runner=runner, resolve=resolve) == 0

    assert resolutions == 1
    assert len(commands) == 1
    command = commands[0]
    assert command[1 : 1 + len(POLICY.scanner_args)] == list(POLICY.scanner_args)
    assert [command[index + 1] for index, value in enumerate(command) if value == "--lockfile"] == list(
        POLICY.lockfiles
    )
    assert "Cargo.lock" not in command


def test_failed_scan_is_never_cached(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.setattr(audit, "load_paths", lambda _root: _paths(tmp_path))
    calls = 0

    def runner(command, **_kwargs):
        nonlocal calls
        calls += 1
        return subprocess.CompletedProcess(command, 1, stdout="finding", stderr="")

    def resolve(_paths, _policy):
        return _tool(tmp_path)

    assert audit.audit_dependencies(PROJECT_ROOT, POLICY, runner=runner, resolve=resolve) == 1
    assert audit.audit_dependencies(PROJECT_ROOT, POLICY, runner=runner, resolve=resolve) == 1
    assert calls == 2
