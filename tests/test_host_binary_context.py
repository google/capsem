"""Selected native executables retain their originating artifact authority."""

import os
import subprocess
import sys
from pathlib import Path


def test_selected_binary_context_survives_per_test_environment_isolation(tmp_path):
    root = Path(__file__).resolve().parents[1]
    environment = dict(os.environ)
    environment["PYTHONPATH"] = str(root / "tests")
    environment["CAPSEM_RELEASE_BIN_DIR"] = str(tmp_path / "selected-binaries")
    environment["CAPSEM_RELEASE_INPUT_DIR"] = str(tmp_path / "selected-inputs")
    subprocess.run(
        [
            sys.executable,
            "-c",
            "from pathlib import Path; import os; "
            "from helpers.constants import host_binary_environment, host_bin_root; "
            "context = host_binary_environment(); "
            "directory = host_bin_root(context); "
            "selected = os.environ.pop('CAPSEM_RELEASE_BIN_DIR'); "
            "inputs = os.environ.pop('CAPSEM_RELEASE_INPUT_DIR'); "
            "assert directory == Path(selected) == host_bin_root(context); "
            "assert context['CAPSEM_RELEASE_INPUT_DIR'] == inputs; "
            "assert 'CAPSEM_RELEASE_INPUT_DIR' not in os.environ",
        ],
        cwd=root,
        env=environment,
        check=True,
        timeout=10,
    )
