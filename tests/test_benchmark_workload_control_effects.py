"""Independent upload/public ledger oracles; synthetic source input only."""
from types import SimpleNamespace
from uuid import UUID

import pytest

from benchmarks.helpers import route_workload_controls as subject
from benchmarks.helpers.route_binary_fixture import verify_import_evidence
from tests.test_benchmark_workload_controls import Backend


@pytest.mark.parametrize('failure',['','write-noop','wrong-bytes','wrong-list','wrong-ack'])
def test_upload_requires_workload_byte_comparison_and_listing(monkeypatch,failure):
    backend = Backend(failure)
    cycle = subject.Cycle(None,backend.target,None,subject.controls_config(),{})
    cycle.owned.add('live')
    monkeypatch.setattr(cycle,'call',backend.call)
    monkeypatch.setattr(cycle,'execute',backend.execute)
    monkeypatch.setattr(subject,'validate_exec',lambda *a:None)
    def upload(vm_id,name,data,*,deadline):
        assert vm_id == 'live' and data == subject.upload_bytes(4096)
        if failure != 'write-noop':
            backend.files['other.txt' if failure == 'wrong-list' else name] = data
        return backend.receipt({'success':failure != 'wrong-ack','size':len(data),
            'vm_path':'/root/'+name,'container_path':'/workspace/'+name})
    monkeypatch.setattr(cycle,'upload',upload)
    if failure:
        with pytest.raises(AssertionError):
            subject.sample_upload(cycle,'live',4096)
    else:
        subject.sample_upload(cycle,'live',4096)
        assert len(cycle.metrics) == 4


@pytest.mark.parametrize('failure',['','counter','missing','duplicate','size','ref','timeline','credential'])
def test_public_events_and_exact_rule_counters_not_echo_are_required(failure):
    event = {'event_id':'abcdef012345','action':'import','path':'route-upload-4096.txt','size':4096,'credential_ref':None}
    rows = [event]
    if failure == 'missing':
        rows = []
    elif failure == 'duplicate':
        rows *= 2
    elif failure in {'size','ref','credential'}:
        event[{'size':'size','ref':'event_id','credential':'credential_ref'}[failure]] = {'size':1,'ref':'bad','credential':'unexpected'}[failure]
    calls = []
    def read(path):
        calls.append(path)
        if path.endswith('/enforcement/status'):
            return {'by_rule':[{'rule_id':'profiles.rules.'+subject.RULE,'rule_action':'allow','count':0 if failure == 'counter' else 1}]}
        if path.endswith('/info'):
            return {'files':{'actions':[{'action':'import','count':1}]}}
        if path.endswith('/stats/detail'):
            return {'file_events':rows}
        return {'events':[] if failure == 'timeline' else [{'layer':'fs','summary':'import route-upload-4096.txt','ref':1}]}
    if failure:
        with pytest.raises(AssertionError):
            verify_import_evidence(read,'live',{'route-upload-4096.txt':4096},subject.RULE,200)
    else:
        verify_import_evidence(read,'live',{'route-upload-4096.txt':4096},subject.RULE,200)
        assert len(calls) == 4


def test_collector_reuses_existing_native_fixture_and_resource_owner(monkeypatch,tmp_path):
    from contextlib import contextmanager

    from helpers import settings_policy

    from benchmarks.helpers import route_lifecycle_fixture
    cfg = subject.controls_config()
    calls,closed = [],[]
    @contextmanager
    def target(label):
        transport = SimpleNamespace(close=lambda:closed.append(label))
        yield SimpleNamespace(label=label,transport_factory=lambda:transport),SimpleNamespace(home_dir=tmp_path,proc=SimpleNamespace(pid=123)),'reference',tmp_path/'cert'
    monkeypatch.setattr(route_lifecycle_fixture,'lifecycle_target',target)
    def rule(*args,**kwargs):
        assert kwargs['match'] == 'file.import.name == "route-upload-4096.txt" || file.import.name == "route-upload-1048576.txt"'
        calls.append('rule')
    monkeypatch.setattr(settings_policy,'write_settings_rule',rule)
    monkeypatch.setattr(subject,'Process',lambda pid:pid)
    monkeypatch.setattr(subject.Cycle,'cleanup',lambda self:None)
    monkeypatch.setattr(subject.Cycle,'listing',lambda self:[])
    def sample(cycle,*args):
        for operation,entry in cfg['endpoints'].items():
            for case in entry['cases']:
                for suffix in ('latency_ms','headers_ms','first_body_byte_ms','verified_completion_ms'):
                    cycle.metrics[f'{cycle.target.label}.{operation} [{case}].{suffix}'] = {'unit':'milliseconds','samples':[1]}
    monkeypatch.setattr(subject,'sample_controls',sample)
    def measure(metrics,phase,root,operation):
        assert root == 123
        calls.append(phase)
        operation()
        metrics[phase+'.wall_s'] = {'unit':'seconds','samples':[1]}
    monkeypatch.setattr(subject,'measure',measure)
    metrics = subject.collect_workload_controls(True)
    assert len(metrics) == 26 and closed == ['service','gateway']
    assert calls == ['rule','service.workload_controls.0','rule','gateway.workload_controls.0']


def test_public_ledger_verification_follows_actual_stop_barrier(monkeypatch,tmp_path):
    from benchmarks.helpers import route_binary_fixture
    cfg,calls = subject.controls_config(),[]
    certificate = tmp_path/'cert'
    certificate.write_text('synthetic certificate input')
    vm_id = str(UUID(int=19))
    def call(method,path,body=None,**kwargs):
        calls.append(path)
        if path == '/vms/create':
            assert isinstance(body,dict)
            assert body['persistent'] and body['container']['image'] == 'reference'
            return None,{'id':vm_id}
        assert path.endswith('/stop') and len(calls) == 5
        return None,{'success':True,'persistent':True}
    cycle = SimpleNamespace(cfg=cfg,listing=lambda **kw:[],call=call,identify=lambda value:value['id'],
        ready=lambda *a,**kw:None,read=lambda *a,**kw:{})
    monkeypatch.setattr(subject,'sample_browser',lambda *a:calls.append('browser'))
    monkeypatch.setattr(subject,'sample_upload',lambda *a:calls.append('upload'))
    def verify(read,ident,expected,rule,limit):
        assert calls[-1].endswith('/stop') and ident == vm_id
        assert expected == {f'route-upload-{size}.txt':size for size in cfg['file_sizes']}
        calls.append('public-ledger')
    monkeypatch.setattr(route_binary_fixture,'verify_import_evidence',verify)
    subject.sample_controls(cycle,'reference',certificate)
    assert calls[-1] == 'public-ledger'
