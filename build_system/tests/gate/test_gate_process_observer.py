"""Foreground observer source-control children; no benchmark/product process."""

import os
import sys
import time

import psutil
import pytest
from capsem_builder.gate import processgroup

POLICY = processgroup.StopPolicy(grace_seconds=1, poll_seconds=0.01)


class Observer:
    def __init__(self, fail=None):
        self.events = []
        self.fail = fail
        self.pid = None
        self.started_at = None

    def event(self, name):
        self.events.append(name)
        if name == self.fail:
            raise ValueError("source callback failed")

    def started(self, process):
        self.pid = process.pid
        self.started_at = psutil.Process(process.pid).create_time()
        self.event("started")

    def before_wait(self, process):
        assert process.returncode is None
        if psutil.pid_exists(process.pid):
            assert psutil.Process(process.pid).create_time() == self.started_at
        self.event("observe")

    def retired(self, process):
        assert process.returncode is not None
        assert not psutil.pid_exists(process.pid)
        self.event("retired")

    def close(self):
        self.event("closed")


def run(
    root,
    observer,
    *,
    script="import time; print('ready',flush=True); time.sleep(.08)",
    timeout=2,
    policy=POLICY,
):
    chunks = []
    status = processgroup.tee(
        [sys.executable, "-c", script],
        cwd=root,
        env=os.environ,
        policy=policy,
        timeout_seconds=timeout,
        write_bytes=chunks.append,
        observer=observer,
    )
    return status, b"".join(chunks)


def test_exact_spawn_observation_and_settled_retirement_order(tmp_path):
    observer = Observer()
    status, output = run(tmp_path, observer)
    assert status == 0 and output == b"ready\n"
    assert observer.events[0] == "started"
    assert observer.events[-2:] == ["retired", "closed"]
    assert "observe" in observer.events


@pytest.mark.parametrize("failure", ["started", "observe", "retired", "closed"])
def test_callback_failure_retires_owned_child_and_closes_observer(tmp_path, failure):
    observer = Observer(failure)
    with pytest.raises(ValueError, match="source callback"):
        run(tmp_path, observer)
    assert observer.events[-1] == "closed"
    assert observer.pid is not None and not psutil.pid_exists(observer.pid)


def test_deadline_retires_observed_child_and_never_claims_success(tmp_path):
    observer = Observer()
    with pytest.raises(Exception, match="timed out"):
        run(tmp_path, observer, script="import time; time.sleep(60)", timeout=0.1)
    assert "retired" not in observer.events and observer.events[-1] == "closed"
    assert not psutil.pid_exists(observer.pid)


def test_live_descendant_prevents_successful_settlement_receipt(tmp_path):
    observer = Observer()
    childfile = tmp_path / "child"
    script = (
        f"import subprocess,sys; p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)']); "
        f"open({str(childfile)!r},'w').write(str(p.pid))"
    )
    with pytest.raises(Exception, match=r"descendant|process group"):
        run(tmp_path, observer, script=script)
    assert "retired" not in observer.events and observer.events[-1] == "closed"
    assert not psutil.pid_exists(observer.pid)
    pid = int(childfile.read_text())
    # macOS can briefly list a dying process after its process group vanished.
    # Independently require this exact source child to settle within the policy.
    expires = time.monotonic() + POLICY.grace_seconds
    while psutil.pid_exists(pid) and time.monotonic() < expires:
        if not processgroup.processstop.is_live(psutil.Process(pid)):
            break
        time.sleep(POLICY.poll_seconds)
    assert not psutil.pid_exists(pid) or not processgroup.processstop.is_live(psutil.Process(pid))


def test_observation_cannot_use_lenient_survivor_cleanup(tmp_path, monkeypatch):
    monkeypatch.setattr(processgroup.subprocess, "Popen", lambda *a, **k: pytest.fail("child"))
    with pytest.raises(ValueError, match="strict"):
        run(tmp_path, Observer(), policy=processgroup.StopPolicy(1, 0.01, False))


@pytest.mark.parametrize("seconds", [0, -1, float("inf"), float("nan")])
@pytest.mark.parametrize("kind", ["tee", "run"])
def test_invalid_deadline_fails_before_any_child(tmp_path, monkeypatch, seconds, kind):
    monkeypatch.setattr(processgroup.subprocess, "Popen", lambda *a, **k: pytest.fail("child"))
    with pytest.raises(ValueError):
        if kind == "tee":
            run(tmp_path, Observer(), timeout=seconds)
        else:
            processgroup.run(
                ["unreachable"],
                cwd=tmp_path,
                env=os.environ,
                capture=False,
                policy=POLICY,
                timeout_seconds=seconds,
            )
