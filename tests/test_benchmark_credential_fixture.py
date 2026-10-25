"""Source-only transport/mock composition; no service, workload or secret store runs."""
import json
import shlex
import time
from contextlib import contextmanager
from types import SimpleNamespace

import pytest

from benchmarks.helpers import route_credential_fixture as subject
from benchmarks.helpers import route_credentials


@pytest.mark.parametrize('failure',['','header','body','duplicate','truncated','negative-delivery'])
def test_workload_uses_only_reference_and_exact_mock_fact_or_refuses(tmp_path,failure):
    transcript = tmp_path/'upstream'
    raw = 'capsem_test_never_guest_source'
    token = route_credentials.reference(raw)
    def execute(vm,command,*,deadline):
        assert vm == 'owned' and deadline > time.monotonic()
        source = shlex.split(command)[2]
        assert raw not in source and token in source
        assert "error.code==403" in source if failure == 'negative-delivery' else "echo['has_authorization']" in source
        row = {'method':'POST','path':'/echo','headers':{'host':subject.FIXTURE_HOST,'authorization':'Bearer '+raw},
            'request_bytes':100,'request_body':'route broker proof'}
        if failure == 'header':
            row['headers']['authorization'] = 'Bearer wrong'
        if failure == 'body':
            row['request_body'] = 'wrong'
        transcript.write_text(json.dumps(row)+'\n')
        if failure == 'duplicate':
            transcript.write_text(transcript.read_text()*2)
        result = {'exit_code':0,'truncated':failure == 'truncated',
            'stdout':{'encoding':'utf8','data':'CAPSEM-UID-MAP 0 100000 65536\nbroker-traffic-verified\n'},
            'stderr':{'encoding':'utf8','data':''}}
        return None,result
    cycle = SimpleNamespace(cfg=route_credentials.credential_config(),execute=execute)
    probe = subject.Traffic(cycle,transcript)
    if failure:
        with pytest.raises((AssertionError,ValueError)):
            probe('owned',token,raw,negative=failure == 'negative-delivery',deadline=time.monotonic()+10)
    else:
        probe('owned',token,raw,negative=False,deadline=time.monotonic()+10)


def test_fixture_uses_existing_before_start_corp_and_owned_teardown(monkeypatch,tmp_path):
    from benchmarks.helpers import route_lifecycle_fixture, route_requests
    events = []
    transport = SimpleNamespace(close=lambda:events.append('transport-close'))
    target = SimpleNamespace(transport_factory=lambda:transport)
    service = SimpleNamespace(home_dir=tmp_path)
    @contextmanager
    def lifecycle(label,*,prepare):
        prepare(service)
        events.append('start-existing-owner')
        try:
            yield target,service,'pinned-reference','certificate'
        finally:
            events.append('stop-existing-owner')
    monkeypatch.setattr(route_lifecycle_fixture,'lifecycle_target',lifecycle)
    monkeypatch.setattr(subject,'start_mock_server',lambda **kw:('owned-mock',{'dns_udp_addr':'127.0.0.1:123','http_addr':'127.0.0.1:456'}))
    monkeypatch.setattr(subject,'stop_process',lambda proc:events.append(('stop',proc)))
    def request(transport,target,method,path,body,**kw):
        assert (method,path,body) == ('PATCH','/plugins/secret_exfiltration/edit',{'mode':'block'})
        return None,{'id':'secret_exfiltration','config':{'mode':'block'}}
    monkeypatch.setattr(route_requests,'json_request',request)
    with pytest.raises(RuntimeError,match='observation'), subject.credential_target('gateway',route_credentials.credential_config()) as fixture:
        assert fixture[:4] == (target,service,'pinned-reference','certificate')
        config = (tmp_path/'corp.toml').read_text()
        assert 'allowed_remote_targets = ["fixture.capsem.test:443"]' in config
        assert 'route_broker_negative_binding' in config
        assert 'mcp.capsem.test:443' in config
        assert 'http.host == "mcp.capsem.test"' in config
        raise RuntimeError('observation failure')
    assert events == ['start-existing-owner','transport-close','stop-existing-owner',('stop','owned-mock')]


@pytest.mark.parametrize('broken',[False,True])
def test_collector_preserves_original_resource_owner_and_transport_cleanup(monkeypatch,broken,tmp_path):
    events = []
    transport = SimpleNamespace(close=lambda:events.append('close'))
    service = SimpleNamespace(proc=SimpleNamespace(pid=123))
    certificate = tmp_path/'cert'
    certificate.write_text('synthetic fixture certificate')
    @contextmanager
    def fixture(label,cfg):
        target = SimpleNamespace(label=label,transport_factory=lambda:transport)
        yield target,service,'pinned',certificate,lambda cycle:None
    class Cycle:
        def __init__(self,transport,target,service,cfg,metrics,traffic):
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
    monkeypatch.setattr(subject,'credential_target',fixture)
    monkeypatch.setattr(route_credentials,'Cycle',Cycle)
    monkeypatch.setattr(route_credentials,'Process',lambda pid:('original',pid))
    def measure(metrics,phase,root,operation):
        assert root == ('original',123)
        if broken:
            raise ValueError('partial sample')
        events.append(phase)
    monkeypatch.setattr(route_credentials,'measure',measure)
    monkeypatch.setattr(route_credentials,'require_credential_metrics',lambda *args:events.append('counts'))
    if broken:
        with pytest.raises(ValueError,match='partial'):
            route_credentials.collect_credentials(True)
        assert events == ['close']
    else:
        route_credentials.collect_credentials(True)
        assert events == ['service.credentials.0','drain','close','gateway.credentials.0','drain','close','counts']


def test_unrelated_dns_fixture_is_explicit_and_default_validation_stays_strict():
    from helpers.content_filter_session import upstream_records
    row = {'kind':'dns','qname':'mcp.capsem.test.','qtype':1,'qclass':1,'rcode':0,
        'source_proto':'udp','request_bytes':10,'response_bytes':20}
    with pytest.raises(AssertionError):
        upstream_records(json.dumps(row),required_hosts=())
    assert upstream_records(json.dumps(row),required_hosts=('mcp.capsem.test',),
        allowed_hosts=('mcp.capsem.test',)) == []
