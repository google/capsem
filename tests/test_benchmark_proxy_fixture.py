"""Source doubles for the existing standalone fixture; no processes launched."""
import json
from types import SimpleNamespace

import pytest
from helpers.http_measurement import MeasuredResponse

from benchmarks.helpers import route_proxy_fixture as subject
from benchmarks.helpers.route_proxies import proxy_config


def fixture(tmp_path,monkeypatch):
    workers = {}
    transport = SimpleNamespace(close=lambda:None)
    target = SimpleNamespace(label='service',transport_factory=lambda:transport,headers={})
    service = SimpleNamespace(tmp_dir=tmp_path,proc=SimpleNamespace(pid=11))
    monkeypatch.setattr(subject,'Process',lambda pid:pid)
    monkeypatch.setattr(subject,'process_tree_snapshot',lambda root:SimpleNamespace(processes=workers))
    transcript = tmp_path/'mock.jsonl'
    probe = subject.Probe(service,target,proxy_config(),transcript)
    value = {'session_id': 'proxy-'+'a'*32,'provider': 'openai','bind': '127.0.0.1','port': 1234,
        'base_url': 'http://127.0.0.1:1234/v1','lease_token': 'private-test-token','lease_expires_unix_ms': 15000}
    directory = tmp_path/'sessions'/value['session_id']
    directory.mkdir(parents=True)
    workers[(12,1.)] = SimpleNamespace(name='capsem-proxy')
    return probe,value,workers,transcript


def test_original_identity_and_materialization_are_independent(tmp_path,monkeypatch):
    probe,value,workers,_ = fixture(tmp_path,monkeypatch)
    owner = probe.capture(value)
    assert owner['identity'] == (12,1.)
    probe.before = set(workers)
    with pytest.raises(AssertionError,match='original owned'):
        probe.capture(value)


@pytest.mark.parametrize('change',[
    {'session_id':'../../foreign'},{'provider':'other'},{'base_url':'http://attacker.invalid/v1'},
    {'port':0},{'lease_token':''},{'lease_expires_unix_ms':0},
])
def test_foreign_or_invalid_receipt_is_refused(tmp_path,monkeypatch,change):
    probe,value,_,_ = fixture(tmp_path,monkeypatch)
    with pytest.raises((AssertionError,ValueError)):
        probe.capture({**value,**change})


def test_missing_or_symlink_session_is_not_materialization(tmp_path,monkeypatch):
    probe,value,_,_ = fixture(tmp_path,monkeypatch)
    directory = tmp_path/'sessions'/value['session_id']
    directory.rmdir()
    with pytest.raises(AssertionError):
        probe.capture(value)
    directory.symlink_to(tmp_path,target_is_directory=True)
    with pytest.raises(AssertionError):
        probe.capture(value)


@pytest.mark.parametrize('failure',['', 'wrong-body','missing-upstream','duplicate-upstream','changed-worker','retry'])
def test_model_probe_uses_strict_owner_and_mock_fact_and_closes(tmp_path,monkeypatch,failure):
    probe,value,workers,transcript = fixture(tmp_path,monkeypatch)
    owner = probe.capture(value)
    closed = []
    def request(method,path,**kwargs):
        assert path == '/v1/chat/completions'
        assert json.loads(kwargs['body'])['stream'] is False
        row = {'method':'POST','path':path,'headers':{'host':'api.openai.com'},'request_body':kwargs['body'].decode()}
        if failure != 'missing-upstream':
            transcript.write_text((json.dumps(row)+'\n')*(2 if failure == 'duplicate-upstream' else 1))
        if failure == 'changed-worker':
            workers.clear()
        content = 'wrong' if failure == 'wrong-body' else probe.cfg['expected_content']
        return MeasuredResponse(200,{},json.dumps({'choices':[{'message':{'content':content}}]}).encode(),1,2,3,attempts=2 if failure == 'retry' else 1)
    transport = SimpleNamespace(measure_request=request,close=lambda:closed.append(True))
    monkeypatch.setattr(subject,'Transport',lambda **kw:transport)
    if failure:
        with pytest.raises((AssertionError,ValueError)):
            probe.model(owner,probe.clock()+10)
    else:
        probe.model(owner,probe.clock()+10)
    assert closed == [True]


def test_record_budget_and_deadline_fail_loudly(tmp_path,monkeypatch):
    probe,_,_,transcript = fixture(tmp_path,monkeypatch)
    probe.cfg['max_log_bytes'] = 5
    transcript.write_bytes(b'abcdef')
    with pytest.raises(AssertionError,match='budget'):
        probe.records()
    with pytest.raises(TimeoutError):
        probe.call('POST','/proxies',{},probe.clock()-1)
    with pytest.raises(TimeoutError):
        probe.wait(probe.clock()+100,probe.clock()+1)


def test_fixture_failure_closes_control_and_service_before_mock(monkeypatch,tmp_path):
    calls = []
    service = SimpleNamespace(home_dir=tmp_path,tmp_dir=tmp_path,start=lambda:calls.append('start'),stop=lambda:calls.append('stop'))
    monkeypatch.setattr(subject,'ServiceInstance',lambda **kw:service)
    monkeypatch.setattr(subject,'start_mock_server',lambda **kw:('mock',{'http_addr':'127.0.0.1:9999'}))
    monkeypatch.setattr(subject,'stop_process',lambda proc:calls.append('mock-stop'))
    monkeypatch.setattr(subject,'clone_benchmark_assets',lambda *a:tmp_path/'assets')
    monkeypatch.setattr(subject,'service_target',lambda *a:None)
    monkeypatch.setattr(subject,'Probe',lambda *a:SimpleNamespace(transport=SimpleNamespace(close=lambda:calls.append('close'))))
    with pytest.raises(RuntimeError), subject.proxy_target('service',proxy_config()):
        raise RuntimeError('fixture failure')
    assert calls == ['start','close','stop','mock-stop']


def test_settlement_requires_listener_worker_and_directory_all_gone(tmp_path,monkeypatch):
    probe,value,workers,_ = fixture(tmp_path,monkeypatch)
    owner = probe.capture(value)
    class Socket:
        def __enter__(self):
            return self
        def __exit__(self,*args):
            pass
        def settimeout(self,value):
            pass
        def connect_ex(self,addr):
            return 1
    monkeypatch.setattr(subject.socket,'socket',Socket)
    monkeypatch.setattr(probe,'clock',lambda:10)
    with pytest.raises(TimeoutError,match='settle'):
        probe.settled(owner,9)
    workers.clear()
    with pytest.raises(TimeoutError,match='settle'):
        probe.settled(owner,9)
    owner['directory'].rmdir()
    probe.settled(owner,11)
