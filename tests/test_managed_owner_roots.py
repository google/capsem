"""Remembered native identities are the only sampler roots, with no commands."""
import time
from types import SimpleNamespace
from typing import Any

import pytest
from helpers import managed_observation, managed_service
from helpers.http_measurement import MeasuredResponse
from helpers.process_snapshot import OwnedProcess


def test_owned_cache_is_stamped_and_snapshot_calls_no_manager(tmp_path,monkeypatch):
    child: Any = SimpleNamespace(pid=12,create_time=lambda:2.,is_running=lambda:True,children=lambda **kwargs:[])
    children = [child]
    root: Any = SimpleNamespace(pid=11,create_time=lambda:1.,is_running=lambda:True,children=lambda **kwargs:children)
    service: Any = SimpleNamespace(tmp_dir=tmp_path)
    owner = managed_service.ManagedService(service)
    with monkeypatch.context() as scoped:
        scoped.setattr(managed_service.psutil,'Process',lambda pid:{11:root,12:child}[pid])
        def command(*args,**kwargs):
            raise AssertionError('sampler root snapshot invoked manager command')
        scoped.setattr(owner,'command',command)
        owner.remember(11)
        assert [(value.pid,value.started_at) for value in owner.owned_roots()] == [(11,1.),(12,2.)]
        assert owner.owned_roots() == owner.owned_roots()
        with pytest.raises(RuntimeError,match='not an original'):
            owner.remember_gateway(11,999)
        with pytest.raises(ValueError,match='global supervisor'):
            owner.remember(1)
        children[:] = [SimpleNamespace(pid=12,create_time=lambda:3.)]
        with pytest.raises(RuntimeError,match='PID replaced'):
            owner.remember(11)
        assert owner.owned_roots()[1].started_at == 2.


def test_new_ready_does_not_erase_live_original_or_skip_reconnection(tmp_path,monkeypatch):
    service: Any = SimpleNamespace(tmp_dir=tmp_path)
    owner = managed_service.ManagedService(service)
    monkeypatch.setattr(owner,'pid',lambda **kwargs:113)
    monkeypatch.setattr(owner,'remember',lambda pid:None)
    monkeypatch.setattr(owner,'remember_gateway',lambda service,gateway:None)
    for name,value in {'gateway.port':'1235','gateway.token':'private-new-token','gateway.pid':'114'}.items():
        (tmp_path/name).write_text(value)
    polls = []
    def live():
        polls.append(1)
        return len(polls) < 3
    original: Any = SimpleNamespace(pid=111,create_time=lambda:1.,is_running=live,status=lambda:'running')
    gateway: Any = SimpleNamespace(pid=112,create_time=lambda:1.,is_running=lambda:False)
    probes = []
    def probe(identity,policy,remaining):
        probes.append(identity)
        return MeasuredResponse(200,{},b'{"sandboxes":[]}',1,2,3)
    monkeypatch.setattr(managed_observation,'strict_probe',probe)
    result = owner.ready_observed(managed_service.GatewayIdentity(1234,'private-old-token',111,112),
        policy=managed_observation.ReadinessPolicy(time.monotonic()+1,.001,1024,4),
        original=(OwnedProcess.capture(original),OwnedProcess.capture(gateway)))
    assert result.outcomes == ('ready','waiting_retirement','reconnected')
    assert len(probes) == 2 and result.ready_ns < result.retired_ns <= result.complete_ns
