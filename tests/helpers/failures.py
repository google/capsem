"""This worker's failed tests, and where their evidence is preserved.

Lives in helpers rather than the root conftest because `conftest` is not a
unique module name: every suite with its own conftest.py shadows it, and the
evidence helper that imported the registry from `conftest` silently
preserved nothing in those suites.
"""

from __future__ import annotations

from pathlib import Path

from capsem_builder.cache.config import load_paths

_PROJECT_ROOT = Path(__file__).resolve().parents[2]

#: Appended by the root conftest's makereport hook; read at teardown.
FAILED_NODEIDS: list[str] = []

#: Homes of the services running right now. A failure preserves them at once:
#: a module-scoped VM fixture deletes its VM, and with it process.log,
#: serial.log and session.db, long before the service's own teardown.
LIVE_HOMES: set[Path] = set()

#: cache/target/tests/evidence/: gitignored, so service.log,
#: sessions/<vm>/process.log, serial.log and session.db survive the rmtree.
def artifact_root(repository_root: Path) -> Path:
    paths = load_paths(repository_root)
    if paths.policy.control is None:
        raise ValueError("cache policy has no failure evidence control")
    return paths.stage(paths.policy.control.failure_artifacts.stage)


ARTIFACTS_ROOT = artifact_root(_PROJECT_ROOT)
