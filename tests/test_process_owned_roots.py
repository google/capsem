"""Resource accounting follows only stamped roots of one disposable owner."""
from dataclasses import replace
from types import SimpleNamespace

import psutil
import pytest
from helpers import process_snapshot as subject
from helpers.process_resources import ProcessTreeSampler
from helpers.process_snapshot import TreeSnapshot

from tests.test_process_resource_sampler import usage


def root(pid,started=1.):
    return SimpleNamespace(pid=pid,create_time=lambda:started)


def test_overlap_is_one_snapshot_and_replacement_is_new_cpu_identity(monkeypatch):
    first,second = root(11),root(12,2.)
    rows = {11:usage(11,'capsem-service',cpu='10',rss=100,pss=80,uss=60,fds=2),
        12:usage(12,'capsem-gateway',cpu='2',rss=50,pss=40,uss=30,fds=1,started_at=2.)}
    def snapshot(process):
        values = [rows[process.pid]]
        if process.pid == 11:
            values.append(rows[12])
        return TreeSnapshot({(value.pid,value.started_at):value for value in values})
    monkeypatch.setattr(subject.psutil,'Process',lambda pid:{11:first,12:second}[pid])
    monkeypatch.setattr(subject,'process_tree_snapshot',snapshot)
    roots = (subject.OwnedProcess.capture(first),subject.OwnedProcess.capture(second))
    combined = subject.owned_process_snapshot(roots,maximum=4)
    assert len(combined.processes) == 2
    sampler = ProcessTreeSampler(None)
    sampler.observe(combined)
    rows[11] = replace(rows[11],cpu_seconds=rows[11].cpu_seconds+1)
    sampler.observe(subject.owned_process_snapshot(roots,maximum=4))
    assert sampler.metrics()['resources.total.cpu_s']['samples'] == [1.]
    assert sampler.metrics()['resources.total.peak_rss_bytes']['samples'] == [150]
    replacement = root(13,3.)
    rows[13] = usage(13,'capsem-service',cpu='3',rss=175,pss=150,uss=120,fds=2,started_at=3.)
    monkeypatch.setattr(subject.psutil,'Process',lambda pid:replacement if pid == 13 else {11:first,12:second}[pid])
    sampler.observe(subject.owned_process_snapshot((subject.OwnedProcess.capture(replacement),),maximum=4))
    assert sampler.metrics()['resources.total.cpu_s']['samples'] == [4.]
    assert sampler.metrics()['resources.total.peak_rss_bytes']['samples'] == [175]


@pytest.mark.parametrize('fault',['forged-pid','forged-start','reused','denied','empty','budget'])
def test_identity_budget_and_accounting_errors_are_not_silent(monkeypatch,fault):
    original = root(11)
    captured = subject.OwnedProcess.capture(original)
    if fault == 'forged-pid':
        captured = replace(captured,pid=12)
    if fault == 'forged-start':
        captured = replace(captured,started_at=3.)
    def current(pid):
        if fault == 'denied':
            raise psutil.AccessDenied(pid)
        return root(pid,2. if fault == 'reused' else 1.)
    monkeypatch.setattr(subject.psutil,'Process',current)
    with pytest.raises((ValueError,RuntimeError,psutil.AccessDenied)):
        subject.owned_process_snapshot(() if fault == 'empty' else (captured,),maximum=0 if fault == 'budget' else 4)


def test_normal_exit_is_empty_not_replacement_and_bounded_provider_is_exclusive(monkeypatch):
    captured = subject.OwnedProcess.capture(root(11))
    def gone(pid):
        raise psutil.NoSuchProcess(pid)
    monkeypatch.setattr(subject.psutil,'Process',gone)
    assert subject.owned_process_snapshot((captured,),maximum=1).processes == {}
    with pytest.raises(ValueError):
        ProcessTreeSampler(root(12),roots=lambda:(captured,),max_roots=1)
    with pytest.raises(ValueError):
        ProcessTreeSampler(None,roots=lambda:(captured,))
    sampler = ProcessTreeSampler(None,roots=lambda:(captured,),max_roots=1)
    with pytest.raises(RuntimeError,match='process'):
        sampler.start()
