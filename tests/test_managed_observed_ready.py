"""Strict supervisor readiness with explicit original retirement; no native job."""
import errno
import json
import time
from types import SimpleNamespace
from typing import Any

import pytest
from helpers import managed_service as subject
from helpers.http_measurement import MeasuredResponse
from helpers.process_snapshot import OwnedProcess


@pytest.mark.parametrize('fault',['none','partial','retry','status','bad-json','unchanged','old-alive','token-repr','budget','reset','timeout','overflow','refused-once'])
def test_ready_uses_strict_receipt_and_original_exit(tmp_path,monkeypatch,fault):
    from helpers import managed_observation
    old = subject.GatewayIdentity(1234,'private-old-token',111,112)
    for name,value in {'gateway.port':'1235','gateway.token':'private-new-token','gateway.pid':'114'}.items():
        (tmp_path/name).write_text(value)
    if fault == 'unchanged':
        (tmp_path/'gateway.token').write_text(old.token)
    service: Any = SimpleNamespace(tmp_dir=tmp_path)
    owner = subject.ManagedService(service)
    monkeypatch.setattr(owner,'pid',lambda **kwargs:113)
    monkeypatch.setattr(owner,'remember',lambda pid:None)
    monkeypatch.setattr(owner,'remember_gateway',lambda service,gateway:None)
    process: Any = SimpleNamespace(pid=111,create_time=lambda:1.,is_running=lambda:fault == 'old-alive',status=lambda:'running')
    gateway: Any = SimpleNamespace(pid=112,create_time=lambda:1.,is_running=lambda:False,status=lambda:'running')
    original = (OwnedProcess.capture(process),OwnedProcess.capture(gateway))
    calls = []
    class Transport:
        def __init__(self,**kwargs):
            pass
        def measure_request(self,method,path,**kwargs):
            calls.append((method,path,kwargs))
            assert kwargs['timeout'] <= 1 and kwargs['max_response_bytes'] == 1024
            if fault in {'reset','timeout','overflow'} or (fault == 'refused-once' and len(calls) == 1):
                cause = ConnectionRefusedError(errno.ECONNREFUSED,'not listening') if fault == 'refused-once' else ValueError('partial response')
                raise ConnectionError('strict attempt failed') from cause
            return MeasuredResponse(503 if fault == 'status' else 200,{},b'bad' if fault == 'bad-json' else json.dumps({'sandboxes':[]}).encode(),1,None if fault == 'partial' else 2,3,2 if fault == 'retry' else 1)
        def close(self):
            pass
    monkeypatch.setattr(managed_observation,'Transport',Transport)
    policy = managed_observation.ReadinessPolicy(deadline=time.monotonic()+1,poll_seconds=0.001,max_response_bytes=1024,max_attempts=2)
    if fault in {'partial','retry','status','bad-json','unchanged','old-alive','budget','reset','timeout','overflow'}:
        if fault == 'budget':
            policy = managed_observation.ReadinessPolicy(deadline=time.monotonic()+1,poll_seconds=0.001,max_response_bytes=1024,max_attempts=1)
            monkeypatch.setattr(owner,'pid',lambda **kwargs: (_ for _ in ()).throw(ValueError('not started')))
        with pytest.raises((AssertionError,ValueError,RuntimeError)):
            owner.ready_observed(old,policy=policy,original=original)
    else:
        observed = owner.ready_observed(old,policy=policy,original=original)
        assert observed.identity.service_pid == 113 and observed.identity.gateway_pid == 114
        assert observed.receipt is not None and observed.receipt.attempts == 1
        assert observed.outcomes == (('connect_refused','ready') if fault == 'refused-once' else ('ready',))
        assert 0 <= observed.retired_ns <= observed.complete_ns and 0 <= observed.ready_ns <= observed.complete_ns
        assert 'private-' not in repr(observed)
        assert len(calls) == (2 if fault == 'refused-once' else 1)


def test_ordinary_ready_keeps_functional_owner_mode(tmp_path,monkeypatch):
    from helpers import managed_observation
    service: Any = SimpleNamespace(tmp_dir=tmp_path)
    owner = subject.ManagedService(service)
    monkeypatch.setattr(owner,'pid',lambda:113)
    monkeypatch.setattr(owner,'remember',lambda pid:None)
    for name,value in {'gateway.port':'1235','gateway.token':'private-new-token','gateway.pid':'114'}.items():
        (tmp_path/name).write_text(value)
    monkeypatch.setattr(subject.GatewayIdentity,'get',lambda *args:(200,{'sandboxes':[]}))
    def unexpected_strict(*args,**kwargs):
        raise AssertionError('ordinary SDK readiness entered strict measurement')
    monkeypatch.setattr(managed_observation,'strict_probe',unexpected_strict)
    assert owner.ready().service_pid == 113


def test_original_fingerprints_required_before_any_probe(tmp_path):
    from helpers.managed_observation import ReadinessPolicy
    service: Any = SimpleNamespace(tmp_dir=tmp_path)
    owner = subject.ManagedService(service)
    with pytest.raises(ValueError,match='fingerprints'):
        owner.ready_observed(subject.GatewayIdentity(1234,'never-render-token',111,112),
            policy=ReadinessPolicy(time.monotonic()+1,.01,1024,2))


def test_policy_budget_validation():
    from helpers.managed_observation import ReadinessPolicy
    for values in ({'deadline':float('nan')},{'poll_seconds':0},{'max_attempts':0},{'max_response_bytes':0}):
        with pytest.raises(ValueError):
            arguments: dict[str,Any] = {'deadline':time.monotonic()+1,'poll_seconds':.01,'max_attempts':2,'max_response_bytes':1024}|values
            ReadinessPolicy(**arguments)
