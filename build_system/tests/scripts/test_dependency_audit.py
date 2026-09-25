"""The maintained dependency scanner is exact, ordered, and fail closed."""

from __future__ import annotations

import json
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


def _scanner(commands: list[list[str]], unfiltered: set[str]):
    """A scanner that is clean with the policy's exceptions and, with none
    applied, reports exactly `unfiltered`."""

    def runner(command, **_kwargs):
        commands.append(command)
        if command[command.index("--config") + 1] == POLICY.config:
            return subprocess.CompletedProcess(command, 0, stdout="", stderr="")
        groups = [{"ids": [advisory], "aliases": []} for advisory in sorted(unfiltered)]
        report = {"results": [{"packages": [{"groups": groups}]}]}
        return subprocess.CompletedProcess(command, 1, stdout=json.dumps(report), stderr="")

    return runner


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

    runner = _scanner(commands, set(_osv_ignored()))
    assert audit.audit_dependencies(PROJECT_ROOT, POLICY, runner=runner, resolve=resolve) == 0
    assert audit.audit_dependencies(PROJECT_ROOT, POLICY, runner=runner, resolve=resolve) == 0

    assert resolutions == 1
    assert len(commands) == 2, "one policy scan and one exception check, then the cached verdict"
    command = commands[0]
    assert command[1 : 1 + len(POLICY.scanner_args)] == list(POLICY.scanner_args)
    assert [command[index + 1] for index, value in enumerate(command) if value == "--lockfile"] == list(
        POLICY.lockfiles
    )
    assert command[command.index("--config") + 1] == POLICY.config


# RustSec is not the only source of Rust advisories. Three rmcp advisories,
# two of them high, were published to GitHub's advisory database and never to
# RustSec, so `cargo audit` stayed green on them while Dependabot flagged main.
# OSV aggregates both, so it scans the Rust lockfile too; `cargo audit` stays
# as the strict RustSec rail beside it.
#: Every lockfile name the policy already scans, plus the package managers it
#: does not use yet, so a lockfile from a new one cannot arrive unscanned.
LOCKFILE_NAMES = {Path(path).name for path in POLICY.lockfiles} | {"package-lock.json", "yarn.lock"}
#: Lockfiles that exist to exercise tooling, not to ship dependencies.
FIXTURE_ROOT = "tests/fixtures/"


def _tracked_lockfiles() -> set[str]:
    listed = subprocess.run(
        ["git", "ls-files"], cwd=PROJECT_ROOT, check=True, capture_output=True, text=True
    ).stdout.splitlines()
    return {
        path
        for path in listed
        if Path(path).name in LOCKFILE_NAMES and not path.startswith(FIXTURE_ROOT)
    }


def test_every_shipped_lockfile_is_scanned() -> None:
    """A lockfile left off the list is a dependency tree nothing audits.

    `Cargo.lock` and the npm MCP server's lockfile were both missing, and
    nothing noticed: the list was hand-kept and nothing compared it to the tree.
    """
    unscanned = _tracked_lockfiles() - set(POLICY.lockfiles)
    assert not unscanned, f"lockfiles no dependency audit scans: {sorted(unscanned)}"


def _osv_entries() -> list[dict]:
    import tomllib

    document = tomllib.loads((PROJECT_ROOT / POLICY.config).read_text(encoding="utf-8"))
    return list(document.get("IgnoredVulns", []))


def _osv_ignored() -> dict[str, str]:
    return {entry["id"]: entry.get("reason", "") for entry in _osv_entries()}


#: An exception is re-checked against upstream at least this often: past its
#: date the scan fails again instead of accepting it forever.
MAX_EXCEPTION_DAYS = 92


def test_every_osv_exception_expires_for_review() -> None:
    import datetime

    now = datetime.datetime.now(datetime.UTC)
    horizon = now + datetime.timedelta(days=MAX_EXCEPTION_DAYS)
    undated = [entry["id"] for entry in _osv_entries() if "ignoreUntil" not in entry]
    assert not undated, f"OSV exceptions without an ignoreUntil review date: {undated}"
    too_far = [entry["id"] for entry in _osv_entries() if entry["ignoreUntil"] > horizon]
    assert not too_far, f"OSV exceptions dated past {MAX_EXCEPTION_DAYS} days: {too_far}"


def test_every_rust_exception_names_a_crate_still_in_the_lock() -> None:
    """A stale exception hides nothing today and whatever lands tomorrow.

    rand's entry outlived the upgrade that fixed it: every rand in the lock
    was already patched, and the ignore stood ready to swallow the next rand
    advisory unreviewed.
    """
    import tomllib

    locked = {package["name"] for package in tomllib.loads((PROJECT_ROOT / "Cargo.lock").read_text())["package"]}
    stale = [
        f"{advisory} ({reason.split()[0]})"
        for advisory, reason in _osv_ignored().items()
        if advisory.startswith("RUSTSEC-") and reason.split()[0] not in locked
    ]
    assert not stale, f"exceptions for crates no longer in Cargo.lock: {stale}"


def test_osv_and_cargo_audit_ignore_the_same_reviewed_advisories() -> None:
    """One reviewed exception list, enforced by both scanners.

    Two lists that drift let an advisory be accepted by one scanner and fail
    the other, or be silently accepted by both after one entry is widened.
    """
    import tomllib

    cargo = tomllib.loads((PROJECT_ROOT / ".cargo/audit.toml").read_text(encoding="utf-8"))
    rust_ignored = set(cargo["advisories"]["ignore"])
    osv_rust_ignored = {advisory for advisory in _osv_ignored() if advisory.startswith("RUSTSEC-")}
    assert osv_rust_ignored == rust_ignored


def test_every_osv_ignore_says_why() -> None:
    missing = [advisory for advisory, reason in _osv_ignored().items() if len(reason.strip()) < 20]
    assert not missing, f"OSV ignores without a reason: {missing}"


def test_the_ignore_list_is_part_of_the_cached_verdict(tmp_path: Path) -> None:
    """Widening the ignore list must rescan, not replay yesterday's clean."""
    lockfiles = audit._lockfiles(PROJECT_ROOT, POLICY)
    before = audit._digest(PROJECT_ROOT, POLICY, lockfiles)
    moved = tmp_path / "root"
    (moved / Path(POLICY.config).parent).mkdir(parents=True)
    for lockfile in POLICY.lockfiles:
        (moved / lockfile).parent.mkdir(parents=True, exist_ok=True)
        (moved / lockfile).write_bytes((PROJECT_ROOT / lockfile).read_bytes())
    (moved / POLICY.config).write_text(
        (PROJECT_ROOT / POLICY.config).read_text(encoding="utf-8") + "\n# widened\n", encoding="utf-8"
    )
    after = audit._digest(moved, POLICY, audit._lockfiles(moved, POLICY))
    assert before != after


def test_an_exception_that_matches_nothing_fails_the_audit(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.setattr(audit, "load_paths", lambda _root: _paths(tmp_path))
    stale = sorted(_osv_ignored())[0]
    commands: list[list[str]] = []
    runner = _scanner(commands, set(_osv_ignored()) - {stale})

    def resolve(_paths, _policy):
        return _tool(tmp_path)

    assert audit.audit_dependencies(PROJECT_ROOT, POLICY, runner=runner, resolve=resolve) == 1
    # Not cached: the next run checks again.
    assert audit.audit_dependencies(PROJECT_ROOT, POLICY, runner=runner, resolve=resolve) == 1
    assert len(commands) == 4


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
