"""Bounded original HTTP fixture wiring; no product process or native VM."""
import json
import time
import tomllib
from concurrent.futures import Future
from contextlib import contextmanager
from threading import Event
from types import SimpleNamespace

import pytest

from benchmarks.helpers import route_ask_fixture as fixture
from benchmarks.helpers import route_asks as subject


def test_private_ask_uses_only_existing_hermetic_override():
    cfg = tomllib.loads(fixture.policy({'http_addr':'127.0.0.1:1234','dns_udp_addr':'127.0.0.1:1235'}))
    rule = cfg['corp']['rules'][fixture.RULE]
    assert rule['action'] == 'ask' and rule['priority'] < cfg['corp']['rules']['allow_content_fixture']['priority']
    assert rule['match'] == 'http.host == "fixture.capsem.test" && tcp.port == "443" && http.path == "/content/echo"'
    assert cfg['network']['upstream_overrides'][fixture.HOST+':443']['dial'] == '127.0.0.1:1234'


def test_transcript_bounds_dns_and_original_bytes(tmp_path):
    path = tmp_path/'upstream.jsonl'
    assert fixture.records(path,128) == []
    row = {'method':'POST','path':'/content/echo','request_bytes':5,'request_body':'proof','headers':{'host':fixture.HOST}}
    path.write_text(json.dumps(row)+'\n')
    assert fixture.records(path,1024) == [row]
    with pytest.raises(AssertionError,match='budget'):
        fixture.records(path,8)
    path.write_text(json.dumps({'kind':'dns','qname':'foreign.example'})+'\n')
    with pytest.raises(AssertionError):
        fixture.records(path,1024)


def test_original_output_is_complete_mapped_json_not_initial_vm():
    value = {'exit_code':0,'truncated':False,'stdout':{'encoding':'utf8','data':'CAPSEM-UID-MAP 0 100000 65536\n'+json.dumps({'status':200,'body':'proof'})},'stderr':{'encoding':'utf8','data':''}}
    def result():
        future = Future()
        future.set_result((None,value))
        return subject.original_result(future,time.monotonic()+1)
    assert result() == {'status':200,'body':'proof'}
    value['truncated'] = True
    with pytest.raises(AssertionError):
        result()
    value['truncated'] = False
    value['stdout']['data'] = 'CAPSEM-UID-MAP 0 0 4294967295\n{}'
    with pytest.raises(AssertionError):
        result()


def test_metrics_require_every_clock_case_target_and_sampler():
    cfg,metrics = subject.ask_config(),{}
    for label in ('service','gateway'):
        for decision in ('approve','deny'):
            prefix = f'{label}.{subject.OPERATION} [{decision}-original].'
            for name in ('latency_ms','headers_ms','first_body_byte_ms','verified_completion_ms','original_to_pending_ms','original_completion_after_decision_ms'):
                metrics[prefix+name] = {'unit':'milliseconds','samples':[1.]}
            metrics[f'{label}.ask_controls.{decision}.0.wall_s'] = {'unit':'seconds','samples':[1.]}
    subject.require_ask_metrics(metrics,cfg,True)
    for key in metrics:
        original = metrics[key]['samples']
        metrics[key]['samples'] = []
        with pytest.raises((AssertionError,ValueError)):
            subject.require_ask_metrics(metrics,cfg,True)
        metrics[key]['samples'] = original


@pytest.mark.parametrize('broken',[False,True])
def test_exact_retained_fixture_resources_and_failure_teardown(monkeypatch,broken):
    events = []
    @contextmanager
    def target(label,cfg):
        try:
            transport = SimpleNamespace(close=lambda:events.append('socket-close'))
            yield SimpleNamespace(transport_factory=lambda:transport),SimpleNamespace(proc=SimpleNamespace(pid=123)),'reference','certificate',list
        finally:
            events.append('service-stop')
    monkeypatch.setattr(subject,'ask_target',target)
    monkeypatch.setattr(subject,'Process',lambda pid:('original',pid))
    def measure(metrics,phase,root,operation):
        assert root == ('original',123)
        assert phase.endswith(('.approve.0','.deny.0'))
        if broken:
            raise ValueError('original rejected')
        events.append(phase)
    monkeypatch.setattr(subject,'measure',measure)
    monkeypatch.setattr(subject,'require_ask_metrics',lambda *args:events.append('counts'))
    if broken:
        with pytest.raises(ValueError,match='original'):
            subject.collect_ask_controls(True)
        assert events == ['socket-close','service-stop']
    else:
        subject.collect_ask_controls(True)
        assert events == [item for label in ('service','gateway') for decision in ('approve','deny')
            for item in (f'{label}.ask_controls.{decision}.0','socket-close','service-stop')]+['counts']


def test_original_worker_owns_distinct_socket_and_never_replays(monkeypatch):
    import shlex
    entered,release = Event(),Event()
    closed,calls = [],[]
    worker_transport = SimpleNamespace(close=lambda:closed.append('worker'))
    target = SimpleNamespace(label='service',transport_factory=lambda:worker_transport)
    class Caller:
        def __init__(self,transport,target,service,cfg,metrics):
            assert transport is worker_transport
            self.owned = set()
        def execute(self,vm_id,command,*,deadline):
            assert self.owned == {vm_id} and deadline > time.monotonic()
            calls.append(command)
            assert command.startswith('python3 -c ')
            compile(shlex.split(command)[2],'<original-ask-stimulus>','exec')
            entered.set()
            assert release.wait(2)
            return None,{'original':True}
    monkeypatch.setattr(subject,'Cycle',Caller)
    def pending(api,vm_id,original,*,timeout):
        assert entered.wait(2) and not original.done() and closed == []
        return {'genuine-original-placeholder':'explicit-unit-double'}
    monkeypatch.setattr(subject,'pending_original_operation',pending)
    def decide(cycle,vm_id,decision,marker,original,listing,records,*,deadline):
        assert entered.is_set() and not original.done() and decision == 'approve'
        release.set()
        assert original.result(timeout=2) == (None,{'original':True})
        assert closed == ['worker']
    monkeypatch.setattr(subject,'decide',decide)
    cycle = SimpleNamespace(cfg=subject.ask_config(),metrics={},target=target,service='isolated',
        call=lambda *args,**kwargs:(None,{'id':'owned'}),identify=lambda value:value['id'],
        listing=lambda **kwargs:[],ready=lambda *args,**kwargs:None,cleanup=lambda:calls.append('cleanup'))
    subject.sample_ask(cycle,'pinned-reference',SimpleNamespace(read_text=lambda:'certificate'),list,'approve')
    assert len(calls) == 2 and calls[-1] == 'cleanup' and closed == ['worker']
