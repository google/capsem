"""Ironbank contract for the capsem-doctor acceptance gate.

The expensive black-box VM run lives in ``test_doctor_ledger.py`` so broad
Ironbank does not boot a second VM for the same proof. This file keeps the
release gate name stable and fails if the real doctor ledger proof stops using
the shared mock server, stops executing ``capsem-doctor`` in the guest, or drops
the major ledger assertions that make the doctor result auditable.
"""

from __future__ import annotations

import ast
from pathlib import Path


DOCTOR_LEDGER = Path(__file__).with_name("test_doctor_ledger.py")
PROJECT_ROOT = Path(__file__).resolve().parents[2]
DIAGNOSTICS_DIR = PROJECT_ROOT / "guest" / "artifacts" / "diagnostics"


def test_capsem_doctor_gate_is_backed_by_full_ledger_proof() -> None:
    source = DOCTOR_LEDGER.read_text(encoding="utf-8")
    tree = ast.parse(source)
    function_names = {
        node.name for node in ast.walk(tree) if isinstance(node, ast.FunctionDef)
    }

    assert "test_capsem_doctor_pays_protocol_and_security_ledger_debt" in function_names
    assert "start_mock_server()" in source
    assert "CAPSEM_MOCK_SERVER_BASE_URL" in source
    assert '"command": (' in source
    assert "capsem-doctor" in source
    assert "/vms/{session_id}/exec" in source

    for table in [
        "net_events",
        "dns_events",
        "mcp_calls",
        "model_calls",
        "tool_calls",
        "fs_events",
        "security_rule_events",
        "substitution_events",
    ]:
        assert f'"{table}"' in source, table

    for route in [
        "/security/latest",
        "/history",
        "/history/counts",
        "/plugins/list",
        "/plugins/dummy_pre_eicar/edit",
        "/plugins/dummy_post_allow/edit",
        "/mcp/default/info",
        "/mcp/servers/list",
    ]:
        assert route in source, route

    dashdash_fast = "--" + "fast"
    smoke_only = "smoke" + "-only"
    presence_only = "presence" + " only"
    for forbidden in [dashdash_fast, smoke_only, presence_only]:
        assert forbidden not in source


def test_capsem_doctor_guest_diagnostics_keep_functional_package_manager_proof() -> None:
    runtimes = (DIAGNOSTICS_DIR / "test_runtimes.py").read_text(encoding="utf-8")

    expected_proofs = [
        "test_pip_install_works",
        "pip install --no-index",
        "capsem-pip-ok",
        "test_uv_pip_install_works",
        "uv pip install --python /root/.venv/bin/python",
        "capsem-uv-wheel-ok",
        "test_npm_install_global_works",
        "npm install -g file:",
        "capsem-npm-ok",
        "test_npm_install_local_works",
        "node -e 'const pkg = require",
        "Works",
        "test_apt_install_works",
        "apt-get install -y -qq",
        "capsem-apt-ok",
        "test_node_execution",
        "JSON.stringify({node: true",
        "test_zstd_roundtrip_works",
        "zstd -q -f",
        "zstd -q -d -f",
        "cmp ",
    ]

    for proof in expected_proofs:
        assert proof in runtimes, proof
