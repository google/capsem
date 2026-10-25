"""Continuous traffic cannot reset the existing collector's receive budget."""
from __future__ import annotations

import runpy
from contextlib import nullcontext
from pathlib import Path
from types import SimpleNamespace

import pytest


def collector() -> dict:
    return runpy.run_path(str(Path(__file__).resolve().parents[1] / "benchmarks/collectors/stream"))


def test_marker_deadline_expires_even_while_nonmatching_frames_arrive(monkeypatch) -> None:
    module = collector()
    ticks = iter((0.0, 0.2, 2.0))
    seen = []
    def received(*_, **__):
        seen.append(True)
        assert len(seen) == 1, "expired deadline must prevent another frame"
        return "stdout", b"unrelated"
    monkeypatch.setitem(module["read_until"].__globals__, "time", SimpleNamespace(monotonic=lambda: next(ticks)))
    monkeypatch.setitem(module["read_until"].__globals__, "next_frame", received)
    with pytest.raises(TimeoutError, match="deadline"):
        module["read_until"](None, b"marker", 1)
    assert len(seen) == 1


def test_exec_refuses_overflow_before_waiting_for_exit(monkeypatch) -> None:
    module = collector()
    monkeypatch.setitem(module["exec_output"].__globals__, "EXEC_BYTES", 4)
    frames = iter((("stdout", b"x" * 5), ("status", {"type": "exit", "code": 0, "truncated": False})))
    seen = []
    def frame(*_, **__):
        seen.append(True)
        return next(frames)
    monkeypatch.setitem(module["exec_output"].__globals__, "open_stream", lambda *_: nullcontext(None))
    monkeypatch.setitem(module["exec_output"].__globals__, "next_frame", frame)
    with pytest.raises(RuntimeError, match="budget"):
        module["exec_output"](None, "vm")
    assert len(seen) == 1


def test_exec_deadline_is_total_not_per_frame(monkeypatch) -> None:
    module = collector()
    ticks = iter((0.0, 0.2, 301.0))
    seen = []
    monkeypatch.setitem(module["exec_output"].__globals__, "open_stream", lambda *_: nullcontext(None))
    monkeypatch.setitem(module["exec_output"].__globals__, "time", SimpleNamespace(monotonic=lambda: next(ticks)))
    def forbidden(*_, **__):
        seen.append(True)
        assert len(seen) == 1, "no receive after total deadline"
        return "stdout", b"x"
    monkeypatch.setitem(module["exec_output"].__globals__, "next_frame", forbidden)
    with pytest.raises(TimeoutError, match="deadline"):
        module["exec_output"](None, "vm")


def test_marker_at_start_of_large_bounded_frame_is_not_lost(monkeypatch) -> None:
    module = collector()
    frames = iter((("stdout", b"done" + b"x" * 8192),))
    monkeypatch.setitem(module["read_until"].__globals__, "next_frame", lambda *_, **__: next(frames))
    assert module["read_until"](None, b"done", 1) == 8196


def test_split_marker_across_frames_survives_bounded_tail(monkeypatch) -> None:
    module = collector()
    frames = iter((("stdout", b"x" * 8192 + b"do"), ("stdout", b"ne" + b"x" * 8192)))
    monkeypatch.setitem(module["read_until"].__globals__, "next_frame", lambda *_, **__: next(frames))
    assert module["read_until"](None, b"done", 1) == 16388
