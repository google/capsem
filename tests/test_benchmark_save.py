"""Explicit persistence effect doubles, no product process or VM launch."""
import time
from types import SimpleNamespace
from typing import Any

import pytest
from helpers.http_measurement import MeasuredResponse

from benchmarks.helpers import route_save as subject

VM = '123e4567-e89b-42d3-a456-426614174000'
BOOT = '123e4567-e89b-42d3-a456-426614174001'
COLD_BOOT = '123e4567-e89b-42d3-a456-426614174002'
GEN = '18446744073709551614'
COLD_GEN = '18446744073709551615'


def output(text):
    return {'exit_code':0,'truncated':False,'stdout':{'encoding':'utf8','data':text},
        'stderr':{'encoding':'utf8','data':''}}


def receipt():
    return MeasuredResponse(200,{},b'{}',1,2,3)


class Backend:
    def __init__(self,root,fault):
        self.cfg,self.fault,self.calls = subject.save_config(),fault,[]
        self.row: dict[str,Any] = {}
        self.path,self.saved,self.cold = root/'not-created',False,False
        self.root,self.original = root,None
        self.processes = {123:True,124:False}
    def call(self,method,path,body=None,*,deadline=None,expected_status=200):
        assert deadline is not None and deadline > time.monotonic()
        self.calls.append((method,path,body))
        if path == '/vms/list':
            value = {'sandboxes':[] if not self.row else [dict(self.row)]}
        elif path == '/vms/create':
            assert body is not None and body['persistent'] is False and 'name' not in body
            self.row = {'id':VM,'name':'vm-1','status':'Running','persistent':self.fault == 'not-ephemeral','pid':123,'can_resume':False}
            self.path = self.root/'sessions'/VM
            self.path.mkdir(parents=True)
            self.original = self.path
            value = {'id':VM}
        elif path.endswith('/info'):
            value = dict(self.row)
        elif path.endswith('/exposures'):
            generation = COLD_GEN if self.cold or (self.saved and self.fault == 'generation-replaced') else GEN
            if self.fault == 'generation-reused':
                generation = GEN
            value = {'owner_generation':generation,'exposures':[]}
        elif path.endswith('/save'):
            assert body == {'name':self.cfg['name']}
            if self.fault != 'noop':
                self.row.update(name=self.cfg['name'],persistent=True,can_resume=True)
            if self.fault == 'replace-owner':
                self.row['pid'] = 124
                self.processes[124] = True
            if self.fault == 'move-live':
                self.move()
            value = {'success':True,'name':self.cfg['name']}
            self.saved = True
        elif path.endswith('/stop'):
            self.processes[self.row['pid']] = self.fault == 'live-stop'
            self.row['status'] = 'Stopped'
            if not self.cold:
                self.move()
            value = {'success':True,'persistent':True}
        elif path.endswith('/start'):
            assert self.saved and self.row['status'] == 'Stopped'
            self.cold = True
            self.row.update(status='Running',pid=124)
            self.processes[124] = True
            value = {'id':VM,'persistent':True}
        elif path.endswith('/exec'):
            assert body is not None
            command = body['command']
            if '/proc/sys/kernel/random/boot_id' in command:
                value = output((BOOT if not self.cold or self.fault == 'warm' else COLD_BOOT)+'\n')
            else:
                assert self.cold and 'test ! -e /run/' in command
                value = output('wrong\n' if self.fault == 'transient-survived' else 'cold-runtime\n')
        elif method == 'DELETE':
            if self.fault != 'delete-noop':
                self.row = {}
                self.path.rmdir()
                self.processes[124] = False
            value = {'success':True}
        else:
            raise AssertionError((method,path))
        measured = receipt()
        if self.fault == 'retry' and path.endswith('/save'):
            measured = MeasuredResponse(200,{},b'{}',1,2,3,attempts=2)
        return measured,value
    def move(self):
        destination = self.root/'persistent'/VM
        destination.parent.mkdir(exist_ok=True)
        if self.fault == 'clone-path':
            destination.mkdir()
        else:
            self.path.rename(destination)
        self.path = destination


def setup(monkeypatch,tmp_path,fault=''):
    backend = Backend(tmp_path,fault)
    cycle = subject.Cycle(None,SimpleNamespace(label='service'),SimpleNamespace(tmp_dir=tmp_path),backend.cfg,{})
    monkeypatch.setattr(cycle,'call',backend.call)
    monkeypatch.setattr(cycle,'session',lambda *args,**kwargs:backend.path)
    def ready(vm,reference,*,deadline,persistent=True):
        assert backend.row['persistent'] is persistent
        return dict(backend.row)
    monkeypatch.setattr(cycle,'ready',ready)
    def execute(vm,command,*,deadline):
        marker = backend.cfg['marker']
        if fault == 'lost-bytes' and backend.cold:
            marker = 'lost'
        mapping = '0 0 4294967295' if fault == 'vm-fallback' else '0 100000 65536'
        return receipt(),output('CAPSEM-UID-MAP '+mapping+'\n'+marker+'\n')
    monkeypatch.setattr(cycle,'execute',execute)
    def process(pid):
        return SimpleNamespace(pid=pid,create_time=lambda:float(pid),is_running=lambda:backend.processes[pid])
    monkeypatch.setattr(subject,'Process',process)
    def independent(*args):
        assert fault != 'wrong-export' and not (backend.cold and fault == 'cold-wrong-export')
    monkeypatch.setattr(subject,'read_file',independent)
    certificate = tmp_path/'cert'
    certificate.write_text('synthetic')
    return cycle,backend,certificate


def test_save_is_original_live_claim_then_normal_stop_cold_retention(monkeypatch,tmp_path):
    cycle,backend,certificate = setup(monkeypatch,tmp_path)
    subject.sample_save(cycle,'pinned',certificate)
    assert not cycle.owned and not backend.row and len(cycle.metrics) == 4
    methods = [(method,path.rsplit('/',1)[-1]) for method,path,_ in backend.calls]
    assert methods.count(('POST','save')) == 1 and methods.count(('POST','start')) == 1
    assert methods.count(('POST','stop')) == 2 and methods.count(('DELETE','delete')) == 1
    cycle.metrics['service.save_controls.0.wall_s'] = {'samples':[1]}
    subject.require_save_metrics(cycle.metrics,cycle.cfg,True,labels=('service',))


@pytest.mark.parametrize('fault',['not-ephemeral','noop','replace-owner','move-live','clone-path',
    'live-stop','warm','lost-bytes','wrong-export','cold-wrong-export','transient-survived','delete-noop','retry','vm-fallback',
    'generation-replaced','generation-reused'])
def test_false_success_lost_identity_or_warm_substitution_cannot_pass(monkeypatch,tmp_path,fault):
    cycle,_,certificate = setup(monkeypatch,tmp_path,fault)
    with pytest.raises((AssertionError,ValueError,FileNotFoundError)):
        subject.sample_save(cycle,'pinned',certificate)


def test_config_requires_one_finite_original_save_case():
    from capsem_builder.gate.benchmarksaveschema import SaveRouteConfig
    cfg = subject.save_config()
    for changed in ({'cycles':5},{'quick_cycles':3},{'timeout_seconds':1},{'name':'../foreign'},{'endpoints':{}},{'marker':''}):
        with pytest.raises(ValueError):
            SaveRouteConfig.model_validate({**cfg,**changed})
    with pytest.raises(ValueError,match='partial'):
        subject.require_save_metrics({},cfg,True)
