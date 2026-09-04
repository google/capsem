"""Audit Python and Node lockfiles with the maintained OSV scanner."""

from __future__ import annotations

import json
import subprocess
import sys
from collections.abc import Callable
from pathlib import Path

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


def _digest(root: Path, policy: DependencyAuditConfig, lockfiles: tuple[Path, ...]) -> str:
    payload = {
        "schema": 1,
        "policy": policy.model_dump(mode="json"),
        "lockfiles": {
            path.relative_to(root).as_posix(): subject_digest(path.read_bytes())
            for path in lockfiles
        },
    }
    return subject_digest(
        json.dumps(payload, separators=(",", ":"), sort_keys=True).encode("utf-8")
    )


def audit_dependencies(
    root: Path,
    policy: DependencyAuditConfig,
    *,
    runner: Run = subprocess.run,
    resolve: Resolve = materialize,
) -> int:
    """Reuse an exact clean verdict or scan every configured lockfile once."""
    cache_paths = load_paths(root)
    lockfiles = _lockfiles(root, policy)
    digest = _digest(root, policy, lockfiles)
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
    command = [str(tool.path), *policy.scanner_args]
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
