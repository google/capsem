import sys
import types
from pathlib import Path

PROJECT_ROOT = Path(__file__).parent.parent
sys.path.insert(0, str(PROJECT_ROOT / "guest" / "artifacts"))


class _StubTable:
    def __init__(self, *args, **kwargs):
        pass

    def add_column(self, *args, **kwargs):
        pass

    def add_row(self, *args, **kwargs):
        pass

    def add_section(self, *args, **kwargs):
        pass


class _StubConsole:
    def __init__(self, *args, **kwargs):
        pass

    def print(self, *args, **kwargs):
        pass


rich_module = types.ModuleType("rich")
rich_table = types.ModuleType("rich.table")
rich_text = types.ModuleType("rich.text")
rich_console = types.ModuleType("rich.console")
rich_table.Table = _StubTable
rich_text.Text = str
rich_console.Console = _StubConsole
sys.modules.setdefault("rich", rich_module)
sys.modules.setdefault("rich.table", rich_table)
sys.modules.setdefault("rich.text", rich_text)
sys.modules.setdefault("rich.console", rich_console)

from capsem_bench import snapshot  # noqa: E402


def test_snapshot_cleanup_unlinks_symlinked_directories(tmp_path, monkeypatch):
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
