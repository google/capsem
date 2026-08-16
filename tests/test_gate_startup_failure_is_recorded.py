"""A gate invocation that dies before its run log exists still leaves one.

`[runlog]` in `config/gate.toml` says a failed gate should be "a directory you
attach to a bug rather than a scrollback you had to be present for", and the
gate contract says the run log is written by the runner "so nothing can be
forgotten into invisibility".

Neither held before the first step runs. Two `release-profile stable co-work`
invocations failed on an unparseable config and left no run directory and no
ledger row -- `target/gate-runs/` still reported the previous run as the last
one, and the only evidence the failures had happened at all was a terminal
scrollback that happened to have been redirected to a file.

That is the most expensive failure to leave unrecorded: it is the one with no
step to point at.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from capsem.gate import cli

PROJECT_ROOT = Path(__file__).resolve().parents[1]
COMMIT = "1" * 40


def _checkout(tmp_path: Path, config_text: str) -> Path:
    root = tmp_path / "checkout"
    (root / "config").mkdir(parents=True)
    (root / "config/gate.toml").write_text(config_text, encoding="utf-8")
    # `project_root()` refuses a tree with no justfile.
    (root / "justfile").write_text("", encoding="utf-8")
    return root


def _records(root: Path) -> list[dict]:
    path = root / "target/gate-runs/startup.jsonl"
    assert path.is_file(), f"no startup failure record at {path}"
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line]


def test_an_unreadable_config_is_recorded_rather_than_only_printed(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The exact failure that left no trace: a config this schema refuses."""
    text = (PROJECT_ROOT / "config/gate.toml").read_text(encoding="utf-8")
    root = _checkout(tmp_path, text + '\n[a_key_this_schema_never_heard_of]\nvalue = "x"\n')
    monkeypatch.setattr(cli, "project_root", lambda: root)

    status = cli.main(["release-profile", "stable", "co-work", COMMIT])

    assert status == 1
    record = _records(root)[-1]
    assert record["invocation"] == [
        "capsem-gate",
        "release-profile",
        "stable",
        "co-work",
        COMMIT,
    ]
    assert record["status"] == "failed"
    assert "gate.toml is invalid" in record["error"]


def test_the_record_survives_a_config_that_is_not_even_toml(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The recorder cannot depend on the file whose unreadability it reports."""
    root = _checkout(tmp_path, "this is not TOML at all [[[\n")
    monkeypatch.setattr(cli, "project_root", lambda: root)

    status = cli.main(["release-binaries", "stable", COMMIT])

    assert status == 1
    assert "not valid TOML" in _records(root)[-1]["error"]


def test_each_failed_invocation_appends_rather_than_replacing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Two failures in a row are two rows; the second must not hide the first."""
    root = _checkout(tmp_path, "not toml [[[\n")
    monkeypatch.setattr(cli, "project_root", lambda: root)

    cli.main(["release-profile", "stable", "co-work", COMMIT])
    cli.main(["release-profile", "stable", "code", COMMIT])

    rows = _records(root)
    assert len(rows) == 2
    assert [row["invocation"][3] for row in rows] == ["co-work", "code"]


def test_a_successful_invocation_writes_no_startup_record(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """This records the gap before the run log, not every run."""
    text = (PROJECT_ROOT / "config/gate.toml").read_text(encoding="utf-8")
    root = _checkout(tmp_path, text)
    monkeypatch.setattr(cli, "project_root", lambda: root)

    # `--help` exits through SystemExit without reaching a command at all.
    with pytest.raises(SystemExit):
        cli.main(["--help"])

    assert not (root / "target/gate-runs/startup.jsonl").exists()
