"""Audit every shipped lockfile -- Rust, Python, Node -- with the maintained OSV scanner."""

from __future__ import annotations

import datetime
import json
import subprocess
import sys
import tempfile
import tomllib
from collections.abc import Callable
from pathlib import Path
from typing import Any

from capsem_builder.cache.config import load_paths
from capsem_builder.cache.tools import MaterializedTool, materialize
from capsem_builder.cache.verdicts import record_clean, reusable, subject_digest
from capsem_builder.gate import project_root
from capsem_builder.gate.buildschema import DependencyAuditConfig
from capsem_builder.gate.config import for_root

Run = Callable[..., subprocess.CompletedProcess[str]]
Resolve = Callable[..., MaterializedTool]


def _lockfiles(root: Path, policy: DependencyAuditConfig) -> tuple[Path, ...]:
    paths = tuple(root / configured for configured in policy.lockfiles)
    missing = [path.relative_to(root).as_posix() for path in paths if not path.is_file()]
    if missing:
        raise ValueError("dependency audit lockfiles are missing: " + ", ".join(missing))
    return paths


def _utc_today() -> datetime.date:
    return datetime.datetime.now(datetime.UTC).date()


def _digest(
    root: Path, policy: DependencyAuditConfig, lockfiles: tuple[Path, ...], day: datetime.date
) -> str:
    payload = {
        "schema": 3,
        # Advisories publish against unchanged lockfiles, so a clean verdict
        # is evidence for its day only. Keyed on inputs alone it was reused
        # for days while four stable dispatches died on fresh advisories.
        "day": day.isoformat(),
        "policy": policy.model_dump(mode="json"),
        "lockfiles": {
            path.relative_to(root).as_posix(): subject_digest(path.read_bytes())
            for path in lockfiles
        },
        # A widened exception list is a different verdict, not a cache hit.
        "config": subject_digest((root / policy.config).read_bytes()),
    }
    return subject_digest(
        json.dumps(payload, separators=(",", ":"), sort_keys=True).encode("utf-8")
    )


def _exception_ids(root: Path, policy: DependencyAuditConfig) -> set[str]:
    document = tomllib.loads((root / policy.config).read_text(encoding="utf-8"))
    return {entry["id"] for entry in document.get("IgnoredVulns", [])}


def _check_scoped_finding(
    root: Path,
    policy: DependencyAuditConfig,
    source: dict[str, Any],
    package: dict[str, Any],
    identifiers: set[str],
    ignored: set[str],
) -> None:
    """An external OSV report may ignore only the exact reviewed Cargo bytes."""
    metadata = package.get("package", {})
    unscoped = (identifiers & ignored) - {scope.advisory for scope in policy.scoped_ignores}
    if metadata.get("ecosystem") == "crates.io" and any(
        not identifier.startswith("RUSTSEC-") for identifier in unscoped
    ):
        raise ValueError("Rust advisory ignores outside the mirrored RustSec list require a scoped artifact review")
    for scope in policy.scoped_ignores:
        if scope.advisory not in identifiers:
            continue
        identity = tuple(metadata.get(key) for key in ("name", "version", "ecosystem"))
        if identity != (scope.package, scope.version, scope.ecosystem):
            raise ValueError(f"scoped advisory ignore {scope.advisory} cannot admit {identity!r}")
        reported = source.get("source", {}).get("path")
        if not isinstance(reported, str):
            raise ValueError(f"scoped advisory ignore {scope.advisory} requires a lockfile source")
        path = Path(reported)
        relative = path.relative_to(root) if path.is_absolute() else path
        if relative.as_posix() not in policy.lockfiles:
            raise ValueError(f"scoped advisory ignore {scope.advisory} names an unconfigured source")
        document = tomllib.loads((root / relative).read_text(encoding="utf-8"))
        selected = [
            entry for entry in document.get("package", [])
            if entry.get("name") == scope.package and entry.get("version") == scope.version
        ]
        if len(selected) != 1 or (
            selected[0].get("source"), selected[0].get("checksum")
        ) != (scope.registry, scope.checksum):
            raise ValueError(f"scoped advisory ignore {scope.advisory} has no exact reviewed Cargo artifact")


def _stale_exceptions(
    root: Path, policy: DependencyAuditConfig, configured: list[str], runner: Run
) -> list[str]:
    """Exceptions that no finding matches: scan again with none applied.

    osv-scanner does not report an ignore that matched nothing, and such an
    entry is worse than useless. rand's outlived the upgrade that fixed it and
    stood ready to accept the next rand advisory without review.
    """
    exceptions = _exception_ids(root, policy)
    if any(scope.advisory not in exceptions for scope in policy.scoped_ignores):
        raise ValueError("scoped advisory reviews require a corresponding dated OSV ignore")
    if not exceptions:
        return []
    with tempfile.TemporaryDirectory() as scratch:
        empty = Path(scratch) / "no-exceptions.toml"
        empty.write_text("", encoding="utf-8")
        at = configured.index("--config")
        command = [*configured[: at + 1], str(empty), *configured[at + 2 :]]
        result = runner(
            command,
            cwd=root,
            check=False,
            capture_output=True,
            text=True,
            timeout=policy.timeout_seconds,
        )
    if result.returncode not in (0, 1):
        raise ValueError(f"unfiltered OSV exception check failed with exit code {result.returncode}")
    found: set[str] = set()
    for source in json.loads(result.stdout or "{}").get("results", []):
        for package in source.get("packages", []):
            identifiers: set[str] = set()
            for group in package.get("groups", []):
                identifiers.update(group.get("ids", []), group.get("aliases", []))
            for vulnerability in package.get("vulnerabilities", []):
                identifiers.add(vulnerability["id"])
                identifiers.update(vulnerability.get("aliases", []))
            _check_scoped_finding(root, policy, source, package, identifiers, exceptions)
            found.update(identifiers)
    return sorted(exceptions - found)


def audit_dependencies(
    root: Path,
    policy: DependencyAuditConfig,
    *,
    runner: Run = subprocess.run,
    resolve: Resolve = materialize,
    today: Callable[[], datetime.date] = _utc_today,
) -> int:
    """Reuse today's exact clean verdict or scan every configured lockfile once."""
    cache_paths = load_paths(root)
    lockfiles = _lockfiles(root, policy)
    digest = _digest(root, policy, lockfiles, today())
    cached = reusable(
        cache_paths,
        stage_id=policy.cache_stage,
        owner="osv-scanner",
        digest=digest,
    )
    if cached is not None:
        for message in cached.messages:
            print(f"{message} [cached]")
        return 0

    tool = resolve(cache_paths, policy.tool)
    command = [str(tool.path), *policy.scanner_args, "--config", policy.config]
    for configured in policy.lockfiles:
        command.extend(("--lockfile", configured))
    result = runner(
        command,
        cwd=root,
        check=False,
        capture_output=True,
        text=True,
        timeout=policy.timeout_seconds,
    )
    if result.returncode != 0:
        sys.stdout.write(result.stdout or "")
        sys.stderr.write(result.stderr or "")
        print(f"OSV-Scanner failed with exit code {result.returncode}", file=sys.stderr)
        return result.returncode

    stale = _stale_exceptions(root, policy, command, runner)
    if stale:
        print(
            "OSV exceptions that match no finding any more; delete them, or they "
            f"will swallow the next advisory for that crate unreviewed: {', '.join(stale)}",
            file=sys.stderr,
        )
        return 1

    origin = "tool cache" if tool.cache_hit else "verified download"
    message = f"OSV-Scanner {policy.tool.version} clean: {len(lockfiles)} lockfiles ({origin})"
    print(message)
    record_clean(
        cache_paths,
        stage_id=policy.cache_stage,
        owner="osv-scanner",
        digest=digest,
        messages=(message,),
    )
    return 0


def main() -> int:
    root = project_root()
    return audit_dependencies(root, for_root(root).audits.dependency_policy)


def entrypoint() -> int:
    try:
        return main()
    except (OSError, subprocess.SubprocessError, ValueError) as error:
        print(f"dependency audit failed: {error}", file=sys.stderr)
        return 127
