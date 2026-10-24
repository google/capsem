"""Physical effect source doubles; never boot or suspend a real VM."""
import time
from types import SimpleNamespace
from typing import Any

import pytest
from helpers.http_measurement import MeasuredResponse

from benchmarks.helpers import route_pause_resume as subject

VM = '123e4567-e89b-42d3-a456-426614174000'
BOOT = '123e4567-e89b-42d3-a456-426614174001'

def test_active_cases_measure_disabled_refusal_not_warm_success():
    cfg = subject.pause_resume_config()
    assert {key:value['cases'] for key,value in cfg['endpoints'].items()} == {
        'POST /vms/{id}/pause': {'disabled-owner-unchanged':'cycles'},
        'POST /vms/{id}/resume': {'disabled-checkpoint-preserved':'cycles'},
    }
    from capsem_builder.gate.benchmarkwarmschema import PauseResumeConfig
    cfg['endpoints']['POST /vms/{id}/pause']['cases'] = {'original-checkpoint':'cycles'}
    with pytest.raises(ValueError, match='refusal pair'):
        PauseResumeConfig.model_validate(cfg)


def measured(retry=False):
    return MeasuredResponse(200,{},b'{}',1,2,3,2 if retry else 1)


@pytest.mark.parametrize('fault',['none','pause-ack','resume-id','status-only','live-original',
    'missing-checkpoint','bad-complete','cold','no-runtime','same-owner','same-generation',
    'wrong-fd','lost-file','retry','timeout','cleanup'])
def test_pause_then_original_warm_resume_requires_effects(monkeypatch,tmp_path,fault):
    cfg = subject.pause_resume_config()
    path = tmp_path/'persistent'/VM
    (path/'owner/terminal').mkdir(parents=True)
    state: dict[str,Any] = {'status':'Running','pid':123,'alive':{123:True,124:False},'restored':False,'calls':[]}
    def info(*args,**kwargs):
        return {'id':VM,'name':cfg['name'],'status':state['status'],'persistent':True,
            'pid':state['pid'],'can_resume':state['status'] == 'Suspended'}
    def call(method,url,body=None,*,deadline=None,expected_status=200):
        assert deadline is not None and deadline > time.monotonic()
        state['calls'].append((method,url))
        value = {'id':VM,'persistent':True}
        if url.endswith('/pause'):
            state['status'] = 'Suspended'
            state['alive'][123] = fault == 'live-original'
            if fault != 'missing-checkpoint':
                (path/'owner/checkpoint.vzsave').write_bytes(b'original-native-checkpoint')
            (path/'owner/checkpoint.vzsave.complete').write_bytes(b'bad' if fault == 'bad-complete' else b'ok\n')
            (path/'owner/terminal/checkpoint.msgpack').write_bytes(b'original-terminal')
            value = {'success':fault != 'pause-ack'}
        elif url.endswith('/resume'):
            state.update(status='Running',pid=123 if fault == 'same-owner' else 124,restored=True)
            state['alive'][state['pid']] = True
            (path/'owner/checkpoint.vzsave.complete').unlink()
            value = {'id':'foreign' if fault == 'resume-id' else VM,'persistent':True}
        elif url.endswith('/stop'):
            state['status'] = 'Stopped'
            state['alive'][state['pid']] = False
            value = {'success':True,'persistent':True}
        elif method == 'DELETE':
            assert fault != 'cleanup','owned deletion did not settle'
            value = {'success':True}
        return measured(fault == 'retry' and url.endswith('/pause')),value
    cycle = subject.Cycle(None,SimpleNamespace(label='service'),SimpleNamespace(tmp_dir=tmp_path),cfg,{})
    monkeypatch.setattr(cycle,'call',call)
    monkeypatch.setattr(cycle,'identify',lambda value:VM)
    def ready(*args,**kwargs):
        assert not state['restored'] or state.get('foreground_proved'), 'fresh exec ran before original retained descriptors'
        return info()
    monkeypatch.setattr(cycle,'ready',ready)
    monkeypatch.setattr(cycle,'info',info)
    monkeypatch.setattr(cycle,'session',lambda *args,**kwargs:path)
    monkeypatch.setattr(cycle,'listing',lambda **kwargs:[] if not state['calls'] else [info()])
    monkeypatch.setattr(cycle,'read',lambda *args,**kwargs:{'status':state['status']})
    monkeypatch.setattr(cycle,'absent',lambda *args,**kwargs:None)
    monkeypatch.setattr(subject,'Process',lambda pid:SimpleNamespace(pid=pid,create_time=lambda:float(pid),is_running=lambda:state['alive'][pid]))
    monkeypatch.setattr(subject,'generation',lambda *args:'1' if not state['restored'] or fault == 'same-generation' else '2')
    def vm_text(*args):
        assert not state['restored'] or state.get('foreground_proved'), 'VM exec ran before original foreground'
        if state['restored'] and fault == 'no-runtime':
            raise AssertionError('warm runtime marker gone')
        return ('123e4567-e89b-42d3-a456-426614174002' if state['restored'] and fault == 'cold' else BOOT)+'\n'
    monkeypatch.setattr(subject,'vm_text',vm_text)
    def read_file(*args):
        assert fault != 'lost-file','preserved bytes differ'
    monkeypatch.setattr(subject,'read_file',read_file)
    class Probe:
        nonce = 'synthetic-warm-nonce'
        def __init__(self,*args):
            pass
        def start(self,deadline):
            assert deadline > time.monotonic()
        def resumed(self,deadline):
            assert fault not in {'wrong-fd','status-only'},'original foreground RAM/cwd/FD response missing'
            assert fault != 'timeout','total deadline exceeded'
            state['foreground_proved'] = True
    monkeypatch.setattr(subject,'WarmProbe',Probe)
    certificate = SimpleNamespace(read_text=lambda:'synthetic')
    if fault == 'none':
        subject.sample_original_warm(cycle,'pinned',certificate)
        assert sum(url.endswith('/pause') for _,url in state['calls']) == 1
        assert sum(url.endswith('/resume') for _,url in state['calls']) == 1
        assert len(cycle.metrics) == 8
    else:
        with pytest.raises((AssertionError,ValueError,TimeoutError)):
            subject.sample_original_warm(cycle,'pinned',certificate)
    assert sum(url.endswith('/pause') for _,url in state['calls']) <= 1
    assert sum(url.endswith('/resume') for _,url in state['calls']) <= 1


def test_finite_config_and_missing_samples_fail_closed():
    from capsem_builder.gate.benchmarkwarmschema import PauseResumeConfig
    cfg = subject.pause_resume_config()
    assert set(cfg['endpoints']) == {'POST /vms/{id}/pause','POST /vms/{id}/resume'}
    for change in ({'cycles':512},{'max_probe_frames':0},{'endpoints':{}},{'max_metadata_bytes':0}):
        with pytest.raises(ValueError):
            PauseResumeConfig.model_validate({**cfg,**change})
    with pytest.raises(ValueError):
        subject.require_pause_metrics({},cfg,True)


@pytest.mark.parametrize('fault',['symlink','empty-native','bad-marker','oversize-terminal'])
def test_checkpoint_is_original_complete_bounded_pair(tmp_path,fault):
    cfg = subject.pause_resume_config()
    owner = tmp_path/'owner'
    (owner/'terminal').mkdir(parents=True)
    native,terminal,marker = owner/'checkpoint.vzsave',owner/'terminal/checkpoint.msgpack',owner/'checkpoint.vzsave.complete'
    native.write_bytes(b'native')
    terminal.write_bytes(b'terminal')
    marker.write_bytes(b'ok\n')
    if fault == 'symlink':
        native.unlink()
        native.symlink_to(terminal)
    elif fault == 'empty-native':
        native.write_bytes(b'')
    elif fault == 'bad-marker':
        marker.write_bytes(b'not-complete')
    else:
        cfg = {**cfg,'max_terminal_checkpoint_bytes':1}
    with pytest.raises(AssertionError):
        subject.checkpoint(tmp_path,cfg)


@pytest.mark.parametrize('broken',[False,True])
def test_collector_retains_existing_owned_fixture_and_closes_on_failure(monkeypatch,broken):
    from contextlib import contextmanager
    calls = []
    @contextmanager
    def fixture(label,*,prepare):
        assert prepare is subject.prepare_saved_session
        calls.append(('enter',label))
        try:
            yield SimpleNamespace(transport_factory=lambda:SimpleNamespace(close=lambda:calls.append('close'))),SimpleNamespace(proc=SimpleNamespace(pid=123)),'pinned','certificate'
        finally:
            calls.append(('stop',label))
    class Cycle:
        def __init__(self,*args):
            pass
        def cleanup(self):
            calls.append('cleanup')
        def listing(self):
            return []
    monkeypatch.setattr(subject,'Cycle',Cycle)
    monkeypatch.setattr(subject,'Process',lambda pid:('existing-service',pid))
    from benchmarks.helpers import route_lifecycle_fixture
    monkeypatch.setattr(route_lifecycle_fixture,'lifecycle_target',fixture)
    def measure(metrics,phase,root,operation):
        assert root == ('existing-service',123) and '.pause_resume.' in phase
        if broken:
            raise ValueError('original warm operation refused')
    monkeypatch.setattr(subject,'measure',measure)
    monkeypatch.setattr(subject,'require_pause_metrics',lambda *args:None)
    if broken:
        with pytest.raises(ValueError):
            subject.collect_pause_resume(True)
        assert calls[-2:] == ['close',('stop','service')]
    else:
        assert subject.collect_pause_resume(True) == {}
        assert calls == [('enter','service'),'cleanup','close',('stop','service'),
            ('enter','gateway'),'cleanup','close',('stop','gateway')]
