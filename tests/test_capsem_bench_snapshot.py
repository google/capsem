import importlib.util
import sys
import types
from pathlib import Path
from types import SimpleNamespace

import pytest

PROJECT_ROOT = Path(__file__).parent.parent
sys.path.insert(0, str(PROJECT_ROOT / "guest" / "artifacts"))


@pytest.fixture
def snapshot(monkeypatch):
    """Load the guest-only MCP dependency without polluting other test imports."""
    fastmcp = types.ModuleType("fastmcp")
    transports = types.ModuleType("fastmcp.client.transports")
    fastmcp.__dict__["Client"] = object
    transports.__dict__["StdioTransport"] = object
    spec = importlib.util.spec_from_file_location(
        "capsem_bench.snapshot_under_test",
        PROJECT_ROOT / "guest/artifacts/capsem_bench/snapshot.py",
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    with monkeypatch.context() as imports:
        imports.setitem(sys.modules, "fastmcp", fastmcp)
        imports.setitem(sys.modules, "fastmcp.client.transports", transports)
        spec.loader.exec_module(module)
    return module


def test_snapshot_cleanup_unlinks_symlinked_directories(tmp_path, monkeypatch, snapshot):
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = tmp_path / "venv-target"
    target.mkdir()
    (target / "keep.txt").write_text("still here")
    (workspace / ".venv").symlink_to(target, target_is_directory=True)
    real_dir = workspace / "dir_0"
    real_dir.mkdir()
    (real_dir / "file.txt").write_text("remove me")

    monkeypatch.setattr(snapshot, "SNAPSHOT_WORKSPACE", str(workspace))

    snapshot.snapshot_cleanup_workspace()

    assert list(workspace.iterdir()) == []
    assert target.is_dir()
    assert (target / "keep.txt").read_text() == "still here"


def test_snapshot_benchmark_reuses_one_mcp_connection(tmp_path, monkeypatch, snapshot):
    calls = []
    clients = []

    class FakeClient:
        def __init__(self, transport):
            clients.append(transport)

        async def __aenter__(self):
            return self

        async def __aexit__(self, *_args):
            return None

        async def call_tool(self, name, arguments):
            calls.append((name, arguments))
            payload = '{"checkpoint": "cp-7"}' if name.endswith("_create") else "{}"
            return SimpleNamespace(
                content=[SimpleNamespace(text=payload)],
                is_error=False,
            )

    class FakeTransport:
        def __init__(self, *, command, args):
            self.command = command
            self.args = args

    monkeypatch.setattr(snapshot, "Client", FakeClient)
    monkeypatch.setattr(snapshot, "StdioTransport", FakeTransport)
    monkeypatch.setattr(snapshot, "SNAPSHOT_WORKSPACE", str(tmp_path / "workspace"))
    monkeypatch.setattr(snapshot, "SNAPSHOT_FILE_COUNTS", [1])

    result = snapshot.snapshot_bench()

    assert len(clients) == 1
    assert [name for name, _ in calls] == [
        "local__snapshots_create",
        "local__snapshots_list",
        "local__snapshots_changes",
        "local__snapshots_revert",
        "local__snapshots_delete",
    ]
    assert calls[-1][1] == {"checkpoint": "cp-7"}
    assert result["1_files"]["delete_ok"] is True
