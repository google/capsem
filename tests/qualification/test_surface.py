"""Graphical images must start a real window with Chromium's sandbox enabled."""

import json
import shlex
from pathlib import Path

import pytest
from helpers.image_session import workload_exec

from tests.ironbank.kingslanding.test_run import wait_for

pytestmark = [pytest.mark.integration, pytest.mark.capability("surface")]
PROBE = Path(__file__).with_name("surface_probe.py").read_text()


def test_graphical_application_has_a_window_and_namespace_sandbox(
    service, candidate, session, tmp_path
):
    client = service.client()
    labels = candidate.config["Labels"]
    assert labels["org.capsem.surface"] == "xpra"
    status = client.get(f"/vms/{session}/container")
    assert status["surface"] == {"kind": "xpra", "port": int(labels["org.capsem.surface.port"])}, status
    evidence = {}

    def window_ready():
        result = workload_exec(client, session, f"python3 -c {shlex.quote(PROBE)}")
        assert result.get("exit_code") == 0, result
        evidence.update(json.loads(result["stdout_text"]))
        (tmp_path / "graphical-surface.json").write_text(json.dumps(evidence, indent=2))
        return any(candidate.expect["window_title"].casefold() in title.casefold()
                   for title in evidence["window_titles"])

    wait_for(window_ready, f"{candidate.name} graphical window", timeout=120)
    processes = evidence["processes"]
    assert processes, evidence
    assert not any(row["sandbox_disabled"] for row in processes), evidence
    # Electron can retain the zygote argv in a forked renderer; do not infer
    # renderer identity from --type alone. Require its namespace sandbox and
    # a real application window, with seccomp/no_new_privs on every observed process.
    assert any(row["kind"] == "zygote" and len(row["nspid"]) > 1 for row in processes), evidence
    assert all(row["seccomp"] == "2" and row["no_new_privs"] == "1" for row in processes), evidence
