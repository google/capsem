"""Starting a thread is not proof that its native watch was registered."""

from __future__ import annotations

import threading
import time
from pathlib import Path

import pytest
from capsem_builder.gate.errors import GateError
from capsem_builder.gate.observation import Watch


def test_watch_waits_for_native_registration_before_source_work(tmp_path: Path, monkeypatch) -> None:
    native = pytest.importorskip("_watchdog_fsevents")
    add_watch = native.add_watch
    registering, permit, entered = (threading.Event() for _ in range(3))
    failures: list[BaseException] = []

    def delayed(*args):
        registering.set()
        assert permit.wait(5), "test did not release native registration"
        add_watch(*args)

    def work():
        try:
            with Watch([tmp_path], source_root=tmp_path) as watch:
                entered.set()
                (tmp_path / "source.toml").write_text("changed")
                deadline = time.monotonic() + 5
                while not watch.faults and time.monotonic() < deadline:
                    time.sleep(0.01)
                assert any(f.reason == "source-tree" for f in watch.faults)
        except BaseException as error:
            failures.append(error)

    monkeypatch.setattr(native, "add_watch", delayed)
    worker = threading.Thread(target=work)
    worker.start()
    try:
        assert registering.wait(5)
        assert not entered.wait(0.1), "source work began before native registration"
    finally:
        permit.set()
        worker.join(timeout=10)
    assert not worker.is_alive(), "watch teardown stranded its owner"
    assert not failures, failures


def test_native_registration_failure_refuses_unobserved_work(tmp_path: Path, monkeypatch) -> None:
    native = pytest.importorskip("_watchdog_fsevents")

    def denied(*_args):
        raise OSError("native registration denied")

    monkeypatch.setattr(native, "add_watch", denied)
    with (
        pytest.raises(GateError, match="native registration denied"),
        Watch([tmp_path], source_root=tmp_path),
    ):
        pytest.fail("work started without filesystem observation")


def test_timed_out_registration_cleans_up_a_late_native_watch(tmp_path: Path, monkeypatch) -> None:
    native = pytest.importorskip("_watchdog_fsevents")
    add_watch, remove_watch = native.add_watch, native.remove_watch
    permit, removed = threading.Event(), threading.Event()
    registered: list[object] = []

    def delayed(*args):
        assert permit.wait(5)
        add_watch(*args)
        registered.append(args[1])

    def remove(watch):
        remove_watch(watch)
        if watch in registered:
            removed.set()

    monkeypatch.setattr(native, "add_watch", delayed)
    monkeypatch.setattr(native, "remove_watch", remove)
    try:
        with (
            pytest.raises(GateError, match="registration timed out"),
            Watch([tmp_path], source_root=tmp_path, observer_timeout=0.02),
        ):
            pytest.fail("timed-out registration admitted work")
    finally:
        permit.set()
    assert removed.wait(5), "late registration leaked its native watch"


@pytest.mark.parametrize("kind", ["created", "moved", "modified"])
def test_directory_notifications_reconcile_children_without_false_history(
    tmp_path: Path, monkeypatch, kind: str,
) -> None:
    from watchdog.events import DirCreatedEvent, DirModifiedEvent, DirMovedEvent

    old = tmp_path / "old.toml"
    old.write_text("unchanged")

    class Observer:
        def schedule(self, handler, _path, *, recursive):
            self.handler = handler

        def start(self):
            pass

        def stop(self):
            pass

        def join(self, *, timeout):
            pass

    observer = Observer()
    monkeypatch.setattr("watchdog.observers.Observer", lambda: observer)
    with Watch([tmp_path], source_root=tmp_path) as watch:
        created = tmp_path / "nested" / "new.toml"
        created.parent.mkdir()
        created.write_text("live mutation")
        event = (
            DirCreatedEvent(str(tmp_path)) if kind == "created"
            else DirModifiedEvent(str(created.parent)) if kind == "modified"
            else DirMovedEvent(str(tmp_path / "gone"), str(tmp_path))
        )
        observer.handler.on_any_event(event)
        assert {f.path for f in watch.faults} == {created}, (
            "a directory-only event must find live children, not unchanged history"
        )
        old.write_text("changed")
        if kind == "modified":
            event = DirModifiedEvent(str(tmp_path))
        observer.handler.on_any_event(event)
        assert {f.path for f in watch.faults} == {created, old}


def test_metadata_survey_catches_dropped_native_events(tmp_path: Path, monkeypatch) -> None:
    pytest.importorskip("_watchdog_fsevents")
    from watchdog.observers.fsevents import FSEventsEmitter

    monkeypatch.setattr(FSEventsEmitter, "events_callback", lambda *_args: None)
    source = tmp_path / "source.toml"
    source.write_text("original")
    output = tmp_path / "cache"
    output.mkdir()
    with Watch([tmp_path], source_root=tmp_path) as watch:
        source.write_text("changed")
        source.write_text("original")
        linked = output / "linked"
        linked.hardlink_to(source)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            reasons = {f.reason for f in watch.faults}
            if {"source-tree", "hardlinked-source"} <= reasons:
                break
            time.sleep(0.01)
        assert {"source-tree", "hardlinked-source"} <= {f.reason for f in watch.faults}


def test_metadata_survey_failure_refuses_work_and_joins_on_exit(tmp_path: Path, monkeypatch) -> None:
    def failed(_roots):
        raise OSError("injected metadata failure")

    with Watch([tmp_path], source_root=tmp_path, survey_interval=0.01) as watch:
        survey = watch._metadata_survey
        monkeypatch.setattr("capsem_builder.gate.metadatasurvey._scan", failed)
        deadline = time.monotonic() + 5
        while watch._refusal is None and time.monotonic() < deadline:
            time.sleep(0.01)
        with pytest.raises(GateError, match="injected metadata failure"):
            watch.checkpoint()
    assert survey is not None and not survey.is_alive()
