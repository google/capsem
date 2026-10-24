"""Explicit source doubles; synthetic credentials never touch a running service."""
import json
from types import SimpleNamespace

import pytest
from helpers.http_measurement import MeasuredResponse

from benchmarks.helpers import route_credentials as subject


class Backend:
    def __init__(self,tmp_path,failure=''):
        self.failure,self.calls,self.cache,self.persisted,self.inventory = failure,[],{}, {},{}
        self.home = tmp_path
        self.cfg = subject.credential_config()
        self.last_count = 0
        self.target = SimpleNamespace(label='service')
        self.service = SimpleNamespace(home_dir=tmp_path)
        self.metrics = {}

    def info(self):
        echo = self.failure == 'reload-count-echo' and self.last_count == 2
        return {'plugin_id':'credential_broker','store':{'backend':'disk_override','ready':True,'status':'ready',
            'last_error':None,'cached_count':3 if echo else len(self.cache),'last_hydrated_count':self.last_count,'last_hydrated_unix_ms':1},
            'inventory':[dict(row,replay_available=ref in self.cache) for ref,row in self.inventory.items()]}

    def call(self,method,path,body=None,*,deadline=None,expected_status=200):
        self.calls.append((method,path))
        if path == '/credentials/inject':
            assert isinstance(body,dict)
            ref = subject.reference(body['value'])
            if self.failure != 'inject-noop':
                self.cache[ref] = body['value']
            if body['storage'] == 'file':
                self.persisted['google:'+ref] = body['value']
                self.write_store(self.persisted)
            value = {'credential_ref':ref,'storage':body['storage']}
        elif path.endswith('/reload'):
            disk = json.loads((self.home/'credential-store.json').read_text())
            if self.failure not in ('reload-noop','reload-count-echo'):
                self.cache.update({key.removeprefix('google:'):value for key,value in disk.items()})
                self.last_count = len(disk)
            if self.failure == 'reload-count-echo':
                self.last_count = len(disk)
            value = self.info()
        else:
            value = self.info()
        return MeasuredResponse(200,{},json.dumps(value).encode(),1,2,3,attempts=2 if self.failure == 'retry' else 1),value

    def write_store(self,values):
        path = self.home/'credential-store.json'
        path.write_text(json.dumps(values))
        path.chmod(0o600)

    def traffic(self,vm,ref,secret,*,negative,deadline):
        assert self.failure != 'wrong-upstream', 'mock original bytes differ'
        if ref not in self.cache:
            disk = json.loads((self.home/'credential-store.json').read_text())
            assert disk['google:'+ref] == secret
        self.inventory[ref] = {'provider':'google','credential_ref':ref,'injected_count':2,'observed_count':1}
        assert self.failure != 'negative-forwarded' or not negative, 'forbidden traffic forwarded'


def setup(monkeypatch,tmp_path,failure=''):
    owner = Backend(tmp_path,failure)
    cycle = subject.Cycle(None,owner.target,owner.service,owner.cfg,{},owner.traffic)
    monkeypatch.setattr(cycle,'call',owner.call)
    return cycle,owner


def test_fresh_injection_and_changed_store_reload_prove_memory_transition(monkeypatch,tmp_path):
    cycle,owner = setup(monkeypatch,tmp_path)
    subject.sample_credentials(cycle,'owned')
    assert len(cycle.metrics) == 12  # two injection cases plus one changed-store reload
    assert sum(path == '/credentials/inject' for _,path in owner.calls) == 2
    assert sum(path.endswith('/reload') for _,path in owner.calls) == 1
    assert len(owner.cache) == 3 and owner.last_count == 2
    assert all('capsem_test_' not in key and 'credential:blake3:' not in key for key in cycle.metrics)


@pytest.mark.parametrize('failure',['inject-noop','reload-noop','reload-count-echo','wrong-upstream','negative-forwarded','retry'])
def test_count_echo_noop_wrong_binding_and_retry_cannot_pass(monkeypatch,tmp_path,failure):
    cycle,_ = setup(monkeypatch,tmp_path,failure)
    with pytest.raises((AssertionError,ValueError,FileNotFoundError,KeyError)):
        subject.sample_credentials(cycle,'owned')


def test_config_and_missing_partial_samples_remain_strict(monkeypatch,tmp_path):
    from capsem_builder.gate.benchmarkcredentialschema import CredentialRouteConfig
    cfg = subject.credential_config()
    for change in ({'cycles':5},{'quick_cycles':3},{'timeout_seconds':1},{'max_store_bytes':999999},{'endpoints':{}}):
        with pytest.raises(ValueError):
            CredentialRouteConfig.model_validate({**cfg,**change})
    with pytest.raises(ValueError,match='partial'):
        subject.require_credential_metrics({},cfg,True)
    cycle,_ = setup(monkeypatch,tmp_path)
    subject.sample_credentials(cycle,'owned')
    cycle.metrics['service.credentials.0.wall_s'] = {'samples':[1]}
    subject.require_credential_metrics(cycle.metrics,cfg,True,labels=('service',))
    cycle.metrics[next(iter(cycle.metrics))]['samples'] *= 2
    with pytest.raises(ValueError,match='partial'):
        subject.require_credential_metrics(cycle.metrics,cfg,True,labels=('service',))
