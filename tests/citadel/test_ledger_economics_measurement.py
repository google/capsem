"""The manual Gemma measurement must fail when an RSS target disappears."""

from __future__ import annotations

import subprocess

import pytest

from tests.manual import ledger_economics


def test_missing_process_pid_cannot_look_like_zero_rss() -> None:
    with pytest.raises(RuntimeError, match="no process PID"):
        ledger_economics.rss_kb([])


def test_exited_process_cannot_look_like_zero_rss(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        ledger_economics.subprocess,
        "run",
        lambda *_args, **_kwargs: subprocess.CompletedProcess(["ps"], 1, "", ""),
    )
    with pytest.raises(RuntimeError, match="RSS unavailable for PID 123"):
        ledger_economics.rss_kb(["123"])
