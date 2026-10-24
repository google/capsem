"""Empty-authority/retained-owner input is explicit, never a real enrollment."""
import tomllib
from types import SimpleNamespace

import pytest

from benchmarks.helpers import route_connection_errors as subject


@pytest.mark.parametrize('failure',['','enabled','supported','connection','running','wrong-owner','grant'])
def test_preflight_proves_auth_disabled_catalog_and_retained_empty_owner(monkeypatch,failure):
    calls = []
    def request(transport,target,method,path,body,**kwargs):
        calls.append(path)
        assert method == 'GET' and body is None
        if path == '/version':
            return None,{'version':'test'}
        if path == '/connections':
            return None,{'connections':['unexpected'] if failure == 'connection' else [],'next_cursor':None,
                'scope_profiles':[{'id':'agy-native','enabled':failure == 'enabled',
                    'capabilities':[{'supported':failure == 'supported'}]}]}
        if path == '/vms/list':
            return None,{'sandboxes':[{'id':'wrong' if failure == 'wrong-owner' else subject.VM_ID,
                'status':'Running' if failure == 'running' else 'Stopped'}]}
        return None,{'session_id':subject.VM_ID,'owner_generation':subject.VM_ID,
                    'grants':['unexpected'] if failure == 'grant' else [],'denied_capability_ids':[]}
    monkeypatch.setattr(subject,'json_request',request)
    if failure:
        with pytest.raises(AssertionError):
            subject.check_empty_authority(None,None,subject.error_config())
    else:
        subject.check_empty_authority(None,None,subject.error_config())
        assert calls == ['/version','/connections','/vms/list',f'/vms/{subject.VM_ID}/credential-grants']


def test_prepare_reuses_stopped_corpus_without_grant_or_credential_seed(monkeypatch,tmp_path):
    calls = []
    service = SimpleNamespace(home_dir=tmp_path,tmp_dir=tmp_path/'run',assets_dir=tmp_path/'assets')
    monkeypatch.setattr(subject,'seed_stopped_workspace',lambda *args,**kwargs:calls.append((args,kwargs)))
    subject.prepare(service)
    assert calls == [((service.tmp_dir,service.assets_dir),{'file_count':1})]
    policy = tomllib.loads((tmp_path/'corp.toml').read_text())
    assert policy == {'refresh_policy':'24h','plugins':{'credential_broker':{'mode':'rewrite','detection_level':'informational'}}}
    assert not list(tmp_path.glob('*credential*')) and not list(tmp_path.glob('*connection*'))


def test_collector_requires_before_after_preflight_and_existing_resources(monkeypatch):
    from contextlib import contextmanager

    from benchmarks.helpers import route_network_fixture
    cfg,calls,closed = subject.error_config(),[],[]
    @contextmanager
    def fixture(label,*,prepare):
        assert prepare is subject.prepare
        yield SimpleNamespace(label=label,transport_factory=lambda:SimpleNamespace(close=lambda:closed.append(label))),SimpleNamespace(proc=SimpleNamespace(pid=123))
    monkeypatch.setattr(route_network_fixture,'network_target',fixture)
    monkeypatch.setattr(subject,'Process',lambda pid:pid)
    monkeypatch.setattr(subject,'check_empty_authority',lambda transport,target,cfg:calls.append(target.label+'.preflight'))
    def samples(transport,target,cfg,*,quick):
        result = {}
        for operation,entry in cfg['endpoints'].items():
            prefix = f"{target.label}.error_path.{operation} [{entry['classification']};http{entry['status']};{entry['code']}]."
            for suffix in ('latency_ms','headers_ms','first_body_byte_ms'):
                result[prefix+suffix] = {'unit':'milliseconds','samples':[1]*cfg['quick_samples']}
        return result
    monkeypatch.setattr(subject,'sample_errors',samples)
    def measure(metrics,phase,root,operation):
        assert root == 123
        calls.append(phase)
        operation()
        metrics[phase+'.wall_s'] = {'unit':'seconds','samples':[1]}
    monkeypatch.setattr(subject,'measure',measure)
    metrics = subject.collect_connection_errors(True)
    assert len(metrics) == 44 and closed == ['service','gateway']
    assert calls == ['service.preflight','service.connection_error_paths','service.preflight',
                     'gateway.preflight','gateway.connection_error_paths','gateway.preflight']
    del metrics['service.connection_error_paths.wall_s']
    with pytest.raises(ValueError,match='resource'):
        subject.require_error_metrics(metrics,cfg,True)


def test_existing_host_fixture_prepares_before_start_and_stops_on_failure(monkeypatch,tmp_path):
    from benchmarks.helpers import route_network_fixture as fixture
    calls = []
    service = SimpleNamespace(home_dir=tmp_path,start=lambda:calls.append('start'),stop=lambda:calls.append('stop'))
    monkeypatch.setattr(fixture,'ServiceInstance',lambda **kwargs:service)
    monkeypatch.setattr(fixture,'clone_benchmark_assets',lambda *a:tmp_path/'assets')
    monkeypatch.setattr(fixture,'service_target',lambda *a:None)
    def prepare(owner):
        assert owner is service and (tmp_path/'settings.toml').exists()
        calls.append('prepare')
        raise RuntimeError('synthetic preparation failure')
    with pytest.raises(RuntimeError),fixture.network_target('gateway',prepare=prepare):
        raise AssertionError('cannot start after preparation failure')
    assert calls == ['prepare','stop']
