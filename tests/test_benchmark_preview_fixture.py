"""Source-only fixture ownership and strict bridge checks, no service/VM launch."""
import json
import time
from contextlib import contextmanager
from types import SimpleNamespace

import pytest
from helpers.http_measurement import MeasuredResponse

from benchmarks.helpers import route_preview as controls
from benchmarks.helpers import route_preview_fixture as subject


def test_registry_exact_admission_and_failure_teardown(monkeypatch,tmp_path):
    from helpers import settings_policy
    events = []
    cfg = controls.preview_config()
    certificate = tmp_path/'certificate'
    certificate.write_text('synthetic')
    class Service:
        def __init__(self,*,isolated_corp_config):
            assert isolated_corp_config is True
            self.home_dir = tmp_path
        def start(self):
            events.append('start')
        def stop(self):
            events.append('stop')
    @contextmanager
    def registry(path,*,image_config):
        assert image_config == {'canonical':'labelled-wiring'}
        try:
            yield 'registry/image@sha256:original',certificate,None
        finally:
            events.append('stop-registry')
    def grant(home,reference,*,certificate):
        assert home == tmp_path and reference == 'registry/image@sha256:original'
        events.append('exact')
    def rule(home,name,**kwargs):
        assert home == tmp_path and kwargs['match'] == 'file.import.name == "preview_server.py"'
        events.append('fixture-only-policy')
    monkeypatch.setattr(subject,'fixture_inputs',lambda:(registry,grant,{'canonical':'labelled-wiring'}))
    monkeypatch.setattr(subject,'ServiceInstance',Service)
    monkeypatch.setattr(subject,'service_target',lambda service,label:label)
    monkeypatch.setattr(settings_policy,'write_settings_rule',rule)
    with pytest.raises(ValueError,match='original sample'), subject.preview_target('service',cfg) as value:
        assert value[0] == 'service' and value[-1] == 'gateway'
        raise ValueError('original sample')
    assert events == ['exact','fixture-only-policy','start','stop','stop-registry']


@pytest.mark.parametrize('fault',['','truncated','foreign-path','wrong-size','retried','unowned'])
def test_bridge_reuses_strict_transport_and_original_upload_identity(fault):
    calls = []
    def measured(method,path,**kwargs):
        calls.append((method,path,kwargs))
        value = {'success':True,'size':99 if fault == 'wrong-size' else 4,
            'vm_path':'/foreign' if fault == 'foreign-path' else '/root/preview_server.py'}
        return MeasuredResponse(200,{},json.dumps(value).encode(),1,2,3,attempts=2 if fault == 'retried' else 1)
    def call(*args,**kwargs):
        return None,{'truncated':fault == 'truncated'}
    cycle = SimpleNamespace(owned=set() if fault == 'unowned' else {'original'},cfg=controls.preview_config(),
        target=SimpleNamespace(headers={}),transport=SimpleNamespace(measure_request=measured),call=call)
    client = subject.Client(cycle,time.monotonic()+10)
    def perform():
        client.upload_file('original','preview_server.py',b'body')
        client.post('/vms/original/exec',{'target':'vm'})
    if fault:
        with pytest.raises((AssertionError,ValueError)):
            perform()
    else:
        perform()
        assert calls[0][:2] == ('POST','/vms/original/files/content?path=preview_server.py')
        assert calls[0][2]['body'] == b'body'


@pytest.mark.parametrize('broken',[False,True])
def test_collector_uses_existing_sampler_finite_cycles_and_closes_both_transports(monkeypatch,broken):
    events = []
    @contextmanager
    def fixture(label,cfg):
        def target(name):
            return SimpleNamespace(label=name,transport_factory=lambda:SimpleNamespace(close=lambda:events.append('close-'+name)))
        yield target(label),SimpleNamespace(proc=SimpleNamespace(pid=123)),'reference',None,target('gateway')
        events.append('owner-stopped')
    class Cycle:
        def __init__(self,*args):
            pass
        def cleanup(self):
            events.append('drain')
        def listing(self):
            return []
    monkeypatch.setattr(subject,'preview_target',fixture)
    monkeypatch.setattr(controls,'Cycle',Cycle)
    monkeypatch.setattr(controls,'Process',lambda pid:('original',pid))
    def measure(metrics,phase,root,operation):
        assert root == ('original',123)
        if broken:
            raise ValueError('partial preview')
        events.append(phase)
    monkeypatch.setattr(controls,'measure',measure)
    monkeypatch.setattr(controls,'require_preview_metrics',lambda *args:events.append('counts'))
    if broken:
        with pytest.raises(ValueError,match='partial'):
            controls.collect_preview_controls(True)
        assert events == ['close-service','close-gateway']
    else:
        controls.collect_preview_controls(True)
        assert events == ['service.preview_controls.0','drain','close-service','close-gateway','owner-stopped',
            'gateway.preview_controls.0','drain','close-gateway','close-gateway','owner-stopped','counts']
