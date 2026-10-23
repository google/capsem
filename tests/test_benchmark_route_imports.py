"""Selected SDK schemas must not replace the repository's fixture namespace."""
from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path


def test_schema_before_workload_fixture_import_uses_repository_namespace() -> None:
    root = Path(__file__).resolve().parents[1]
    result = subprocess.run(
        [sys.executable, "-c", "import benchmarks.helpers.route_validation; import helpers.image_session; import tests.fixtures; print(list(tests.fixtures.__path__)[0])"],
        cwd=root, env={**os.environ, "PYTHONPATH": str(root) + os.pathsep + str(root / "tests")},
        stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=10, check=False,
    )
    assert result.returncode == 0, result.stderr
    assert Path(result.stdout.strip()).resolve() == root / "tests/fixtures"
