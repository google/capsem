"""Source-only original process/upstream/policy owner seams, no native execution."""
import json
import time
from contextlib import contextmanager
from types import SimpleNamespace

import pytest

from benchmarks.helpers import route_mcp
from benchmarks.helpers import route_mcp_fixture as subject


@pytest.mark.parametrize('fault',['','wrong-query','duplicate','body','replaced'])
def test_original_owner_and_nonce_bound_upstream_bytes(monkeypatch,tmp_path,fault):
    cfg = route_mcp.mcp_config()
    cycle = SimpleNamespace(cfg=cfg,info=lambda vm,**kw:{'id':vm,'status':'Running','persistent':True,'pid':123})
    process = SimpleNamespace(pid=123,create_time=lambda:1.,is_running=lambda:True)
    monkeypatch.setattr(subject,'Process',lambda pid:process)
    transcript = tmp_path/'upstream'
    probe = subject.Probe(cycle,transcript)
    probe.ready('owned',deadline=time.monotonic()+2)
    before = probe.before()
    row = {'method':'GET','path':'/html/about','query':'route_mcp=original',
        'headers':{'host':'127.0.0.1:3713'},'request_bytes':0,'request_body':''}
    if fault == 'wrong-query':
        row['query'] = 'cached'
    if fault == 'body':
        row['request_body'] = 'unexpected'
    if fault == 'replaced':
        process.create_time = lambda:2.
    transcript.write_text(json.dumps(row)+'\n')
    if fault == 'duplicate':
        transcript.write_text(transcript.read_text()*2)
    if fault:
        with pytest.raises((AssertionError,ValueError)):
            probe.ready('owned',deadline=time.monotonic()+2)
            probe.delivered(before,'http://127.0.0.1:3713/html/about?route_mcp=original',deadline=time.monotonic()+2)
    else:
        probe.delivered(before,'http://127.0.0.1:3713/html/about?route_mcp=original',deadline=time.monotonic()+2)


def test_fixture_grants_only_two_local_tools_before_owned_boot(monkeypatch,tmp_path):
    from benchmarks.helpers import route_lifecycle_fixture, route_requests
    events = []
    transport = SimpleNamespace(close=lambda:events.append('close'))
    target = SimpleNamespace(transport_factory=lambda:transport)
    service = SimpleNamespace(home_dir=tmp_path)
    @contextmanager
    def lifecycle(label,*,prepare):
        prepare(service)
        try:
            yield target,service,'reference','certificate'
        finally:
            events.append('stop-service')
    monkeypatch.setattr(route_lifecycle_fixture,'lifecycle_target',lifecycle)
    monkeypatch.setattr(subject,'start_mock_server',lambda **kw:('mock',{'base_url':'http://127.0.0.1:3713'}))
    monkeypatch.setattr(subject,'stop_process',lambda process:events.append('stop-mock'))
    calls = []
    def request(transport,target,method,path,body,**kw):
        calls.append((method,path,body))
        return None,{'server_id':'local','tool_id':path.split('/')[-2],'action':body['action'],'mutation':{'status':'applied'}}
    monkeypatch.setattr(route_requests,'json_request',request)
    with pytest.raises(RuntimeError,match='sample'), subject.mcp_target('service',route_mcp.mcp_config()) as value:
        assert value[:4] == (target,service,'reference','certificate')
        config = (tmp_path/'corp.toml').read_text()
        assert 'tcp.port == "3713"' in config and 'http.path == "/html/about"' in config and 'http.method == "GET"' in config
        raise RuntimeError('sample failed')
    assert calls == [('PATCH','/mcp/servers/local/tools/echo/edit',{'action':'block'}),
        ('PATCH','/mcp/servers/local/tools/grep_http/edit',{'action':'block'}),
        ('PATCH','/mcp/servers/local/tools/http_headers/edit',{'action':'allow'}),
        ('PATCH','/mcp/servers/local/tools/fetch_http/edit',{'action':'allow'})]
    assert events == ['close','stop-service','stop-mock']


@pytest.mark.parametrize('broken',[False,True])
def test_collector_preserves_finite_counts_sampler_and_failure_cleanup(monkeypatch,broken,tmp_path):
    events = []
    transport = SimpleNamespace(close=lambda:events.append('close'))
    service = SimpleNamespace(proc=SimpleNamespace(pid=123))
    certificate = tmp_path/'cert'
    certificate.write_text('synthetic')
    @contextmanager
    def fixture(label,cfg):
        target = SimpleNamespace(label=label,transport_factory=lambda:transport)
        yield target,service,'pinned',certificate,lambda cycle:None,'http://127.0.0.1:3713'
    class Cycle:
        def __init__(self,*args):
            pass
        def listing(self,**kwargs):
            return []
        def call(self,*args,**kwargs):
            return None,{'id':'owned'}
        def identify(self,value):
            return value['id']
        def ready(self,*args,**kwargs):
            pass
        def cleanup(self):
            events.append('drain')
    monkeypatch.setattr(subject,'mcp_target',fixture)
    monkeypatch.setattr(route_mcp,'Cycle',Cycle)
    monkeypatch.setattr(route_mcp,'Process',lambda pid:('original-root',pid))
    def measure(metrics,phase,root,operation):
        assert root == ('original-root',123)
        if broken:
            raise ValueError('partial MCP sample')
        events.append(phase)
    monkeypatch.setattr(route_mcp,'measure',measure)
    monkeypatch.setattr(route_mcp,'require_mcp_metrics',lambda *args:events.append('counts'))
    if broken:
        with pytest.raises(ValueError,match='partial'):
            route_mcp.collect_mcp_controls(True)
        assert events == ['close']
    else:
        route_mcp.collect_mcp_controls(True)
        assert events == ['service.mcp_controls.0','drain','close','gateway.mcp_controls.0','drain','close','counts']
