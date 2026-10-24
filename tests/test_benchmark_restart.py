"""Native restart source contracts; doubles never enter the supervisor."""
import json
from types import SimpleNamespace
from typing import cast

import pytest
from helpers.http_measurement import MeasuredResponse

from benchmarks.helpers import route_restart as subject


def receipt(status=200,body=None):
    return MeasuredResponse(status,{},json.dumps(body or {'sandboxes':[]}).encode(),1,2,3,1)


def test_restart_config_is_finite_native_and_not_quick():
    from capsem_builder.gate.benchmarkrestartschema import RestartRouteConfig
    cfg = subject.restart_config()
    assert cfg['dimension'] == 'restart' and cfg['cycles'] <= 4
    assert set(cfg['endpoints']) == {subject.OPERATION}
    assert cfg['endpoints'][subject.OPERATION]['collector'] == 'restart'
    for update in ({'cycles':512},{'max_roots':0},{'max_response_bytes':0},{'endpoints':{}},{'dimension':'routes'}):
        with pytest.raises(ValueError):
            RestartRouteConfig.model_validate({**cfg,**update})


@pytest.mark.parametrize('fault',['none','manager','accepted','same-pid','same-token','no-receipt','old-token','inventory','file','late'])
def test_restart_requires_new_identity_and_independent_original_effect(tmp_path,monkeypatch,fault):
    cfg = subject.restart_config()
    service = SimpleNamespace(tmp_dir=tmp_path,uds_path=tmp_path/'service.sock')
    root = tmp_path/'persistent'/subject.VM_ID/'guest/workspace'
    root.mkdir(parents=True)
    for name,data in subject.workspace_expected(cfg).items():
        (root/name).write_bytes(data)
    (tmp_path/'persistent_registry.json').write_text('{"owned":true}')
    before = SimpleNamespace(service_pid=10,gateway_pid=11,token='before',port=200)
    after = SimpleNamespace(service_pid=10 if fault == 'same-pid' else 20,gateway_pid=21,
        token='before' if fault == 'same-token' else 'after',port=201)
    calls = []
    observation = SimpleNamespace(identity=after,receipt=None if fault == 'no-receipt' else receipt(),
        ready_ns=2,retired_ns=1,complete_ns=3,outcomes=('ready',))
    def ready(previous,**kwargs):
        assert previous is before and kwargs['original'] == ('owned',)
        if fault == 'file':
            (root/'created.txt').write_bytes(b'changed')
        return observation
    owner = SimpleNamespace(ready_observed=ready,owned_roots=lambda:('owned',))
    def request(transport,target,method,path,body,**kwargs):
        calls.append((method,path,kwargs['expected_status']))
        if method == 'POST':
            return receipt(202),{'status':'rejected' if fault == 'accepted' else 'accepted','manager':'wrong' if fault == 'manager' else subject.manager_name(),'authentication':'new_token_required'}
        if kwargs['expected_status'] == 401:
            assert fault != 'old-token', 'old token still authorizes'
            return receipt(401),{'error':'unauthorized'}
        return receipt(),{'sandboxes':['changed'] if fault == 'inventory' else []}
    monkeypatch.setattr(subject,'json_request',request)
    monkeypatch.setattr(subject,'Transport',lambda **kwargs:SimpleNamespace(close=lambda:None))
    if fault == 'late':
        cfg = {**cfg,'timeout_seconds':1e-12}
    if fault == 'none':
        metrics = subject.sample_restart(owner,service,before,{'sandboxes':[]},cfg,'gateway')
        assert len([call for call in calls if call[0] == 'POST']) == 1
        assert any('original_retired_ms' in key for key in metrics)
    else:
        with pytest.raises((AssertionError,ValueError)):
            subject.sample_restart(owner,service,before,{'sandboxes':[]},cfg,'gateway')


def test_restart_metrics_reject_partial_samples():
    cfg = subject.restart_config()
    with pytest.raises(ValueError):
        subject.require_restart_metrics({},cfg)


@pytest.mark.parametrize('fault',['none','seed','forced-cleanup'])
def test_existing_supervisor_lifetime_and_seed_failure_cleanup(monkeypatch,fault):
    from contextlib import contextmanager
    events = []
    service = SimpleNamespace(tmp_dir='isolated',stop=lambda:events.append('service-stop'))
    monkeypatch.setattr(subject,'ServiceInstance',lambda **kwargs:service)
    def seed(*args,**kwargs):
        events.append('seed')
        if fault == 'seed':
            raise ValueError('fixture input missing')
    @contextmanager
    def native(original):
        assert original is service
        events.append('owner-enter')
        try:
            yield 'original'
        finally:
            events.append('owner-retire')
            if fault == 'forced-cleanup':
                raise ValueError('fixture companions required forced cleanup')
    monkeypatch.setattr(subject,'seed_stopped_workspace',seed)
    monkeypatch.setattr(subject,'managed_service',native)
    def run():
        with subject.restart_owner(subject.restart_config()) as (owner,current):
            assert owner == 'original' and current is service
    if fault == 'none':
        run()
    else:
        with pytest.raises(ValueError):
            run()
    assert events[-1] == 'service-stop'
    assert ('owner-enter' in events) is (fault != 'seed')


def test_measure_keeps_existing_fixed_root_and_native_owned_provider(monkeypatch):
    from benchmarks.helpers import stream_measurements
    seen = []
    class Sampler:
        def __init__(self,*args,**kwargs):
            seen.append((args,kwargs))
        def start(self):
            pass
        def stop(self):
            return {}
    monkeypatch.setattr(stream_measurements,'ProcessTreeSampler',Sampler)
    stream_measurements.measure({},'fixed','root',lambda:None)
    def provider():
        return ('original','replacement')
    stream_measurements.measure({},'native',None,lambda:None,roots=provider,max_roots=8)
    assert seen == [(('root',),{}),((None,),{'roots':provider,'max_roots':8})]


def test_native_fixture_disables_background_catalog_and_updates(tmp_path):
    import tomllib

    from helpers.managed_service import service_environment
    from helpers.service import ServiceInstance
    env = service_environment(cast(ServiceInstance,SimpleNamespace(home_dir=tmp_path,tmp_dir=tmp_path)))
    document = tomllib.loads((tmp_path/'settings.toml').read_text())
    assert document['images']['catalog'] is False
    assert document['settings']['app.auto_update']['value'] is False
    assert env['CAPSEM_CREDENTIAL_STORE_PATH'] == str(tmp_path/'credential-store.json')


def test_native_entry_refuses_quick_before_any_fixture(monkeypatch):
    import runpy
    import sys
    def forbidden():
        raise AssertionError('quick must not prepare native resources')
    monkeypatch.setattr(subject,'collect_restart',forbidden)
    monkeypatch.setattr(sys,'argv',['restart','--quick'])
    namespace = runpy.run_path(str(subject.Path(__file__).parents[1]/'benchmarks/collectors/restart'))
    with pytest.raises(SystemExit) as error:
        namespace['main']()
    assert error.value.code == 2


def test_complete_native_metric_membership_rejects_each_partial_series():
    cfg,metrics = subject.restart_config(),{}
    for label in ('service','gateway'):
        prefix = f'{label}.{subject.OPERATION} [managed-original].'
        for name in ('latency_ms','headers_ms','first_body_byte_ms',*subject.CLOCKS,*subject.PROBES):
            metrics[prefix+name] = {'unit':'milliseconds','samples':[1.]*cfg['cycles']}
        metrics[prefix+'readiness_observations'] = {'unit':'count','samples':[1.]*cfg['cycles']}
        for index in range(cfg['cycles']):
            metrics[f'{label}.restart_controls.{index}.wall_s'] = {'unit':'seconds','samples':[1.]}
    subject.require_restart_metrics(metrics,cfg)
    for key in metrics:
        changed = dict(metrics)
        del changed[key]
        with pytest.raises(ValueError):
            subject.require_restart_metrics(changed,cfg)
