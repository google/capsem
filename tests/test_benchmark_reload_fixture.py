"""Existing owning fixture, original traffic facts and finite resource evidence."""
import ast
import shlex
import time
from contextlib import contextmanager
from types import SimpleNamespace

import pytest

from benchmarks.helpers import route_reload as subject


@pytest.mark.parametrize('allowed',[False,True])
@pytest.mark.parametrize('fault',['none','delivered-block','replay','wrong-body','truncated','initial-vm'])
def test_original_http_exact_independent_upstream_or_refuses(allowed,fault):
    rows = []
    def execute(vm_id,command,*,deadline):
        assert vm_id == 'original' and deadline > time.monotonic()
        source = shlex.split(command)[2]
        compile(source,'<original-reload-http>','exec')
        marker = next(ast.literal_eval(node.value) for node in ast.parse(source).body
            if isinstance(node,ast.Assign) and any(isinstance(target,ast.Name) and target.id == 'marker' for target in node.targets))
        assert 'response.read(1048576+1)' in source
        if allowed or fault == 'delivered-block':
            rows.append({'method':'POST','path':'/content/echo','headers':{'host':subject.HOST},
                'request_body':marker if fault != 'wrong-body' else 'other'})
        if fault == 'replay' and rows:
            rows.append(rows[0])
        mapping = '0 0 4294967295' if fault == 'initial-vm' else '0 100000 65536'
        return None,{'exit_code':0,'truncated':fault == 'truncated','stdout':{'encoding':'utf8','data':f'CAPSEM-UID-MAP {mapping}\nreload-http-effect-verified\n'},'stderr':{'encoding':'utf8','data':''}}
    cycle = SimpleNamespace(cfg=subject.reload_config(),execute=execute)
    probe = subject.Probe(cycle,lambda:rows)
    fails = fault in {'truncated','initial-vm'} or (allowed and fault in {'replay','wrong-body'}) or (not allowed and fault == 'delivered-block')
    if fails:
        with pytest.raises(AssertionError):
            probe('original',allowed=allowed,deadline=time.monotonic()+1)
    else:
        probe('original',allowed=allowed,deadline=time.monotonic()+1)
        assert len(rows) == int(allowed)


def test_active_policy_rejects_oversize_symlink_and_parse_error(tmp_path):
    path = tmp_path/'vm'
    path.mkdir()
    artifact = path/'active_policy.toml'
    cycle = SimpleNamespace(cfg={'max_policy_bytes':16},session=lambda *args,**kwargs:tmp_path)
    for raw,error in ((b'x'*17,AssertionError),(b'bad = [',ValueError)):
        artifact.write_bytes(raw)
        with pytest.raises(error):
            subject.active_policy(cycle,'owned',time.monotonic()+1)
    artifact.unlink()
    original = tmp_path/'unrelated'
    original.write_text('a=1')
    artifact.symlink_to(original)
    with pytest.raises(AssertionError):
        subject.active_policy(cycle,'owned',time.monotonic()+1)
    assert original.read_text() == 'a=1'


def test_config_rejects_mutation_loops_wrong_case_and_missing_clock():
    from capsem_builder.gate.benchmarkreloadschema import ReloadRouteConfig
    cfg = subject.reload_config()
    for damaged in ({**cfg,'cycles':512},{**cfg,'quick_cycles':cfg['cycles']+1},
        {**cfg,'endpoints':{}},{**cfg,'max_policy_bytes':0}):
        with pytest.raises(ValueError):
            ReloadRouteConfig.model_validate(damaged)
    metrics = {}
    for label in ('service','gateway'):
        for case in ('block-original','restore-original'):
            for name in ('latency_ms','headers_ms','first_body_byte_ms','verified_completion_ms'):
                metrics[f'{label}.{subject.OPERATION} [{case}].{name}'] = {'unit':'milliseconds','samples':[1.]}
        metrics[f'{label}.reload_controls.0.wall_s'] = {'unit':'seconds','samples':[1.]}
    subject.require_reload_metrics(metrics,cfg,True)
    for key in metrics:
        original = metrics[key]['samples']
        metrics[key]['samples'] = []
        with pytest.raises(ValueError):
            subject.require_reload_metrics(metrics,cfg,True)
        metrics[key]['samples'] = original


@pytest.mark.parametrize('broken',[False,True])
def test_reuses_original_fixture_without_ask_or_native_preparation(monkeypatch,broken):
    events = []
    @contextmanager
    def fixture(label,cfg,*,policy_factory):
        assert policy_factory is subject.fixture_config
        try:
            transport = SimpleNamespace(close=lambda:events.append('socket-close'))
            yield SimpleNamespace(transport_factory=lambda:transport),SimpleNamespace(proc=SimpleNamespace(pid=123)),'pinned','certificate',list
        finally:
            events.append('stop-owner')
    monkeypatch.setattr(subject,'ask_target',fixture)
    monkeypatch.setattr(subject,'Process',lambda pid:('original',pid))
    def measure(metrics,phase,root,operation):
        assert root == ('original',123) and phase.endswith('reload_controls.0')
        if broken:
            raise ValueError('failed original acknowledgement')
        events.append(phase)
    monkeypatch.setattr(subject,'measure',measure)
    monkeypatch.setattr(subject,'require_reload_metrics',lambda *args:events.append('counts'))
    if broken:
        with pytest.raises(ValueError,match='acknowledgement'):
            subject.collect_reload_controls(True)
        assert events == ['socket-close','stop-owner']
    else:
        subject.collect_reload_controls(True)
        assert events == ['service.reload_controls.0','socket-close','stop-owner',
            'gateway.reload_controls.0','socket-close','stop-owner','counts']
