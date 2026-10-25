"""One original request and independent effects; explicit doubles, no VM."""
import json
import threading
from types import SimpleNamespace

import pytest
from helpers.http_measurement import MeasuredResponse

from benchmarks.helpers import route_run_probe as subject
from benchmarks.helpers import route_run_purge as controls


def receipt(body=b'{}',status=200,attempts=1):
    return MeasuredResponse(status,{},body,1,2,3,attempts=attempts)


class Backend:
    def __init__(self,tmp_path,cfg,fault):
        self.cfg,self.fault = cfg,fault
        self.root = tmp_path/'sessions'
        self.path = self.root/'vm-1'
        self.rows,self.released,self.calls,self.alive = [],threading.Event(),[],False
    def call(self,method,path,body=None,*,deadline=None,expected_status=200):
        self.calls.append((method,path))
        if path == '/vms/list':
            value = {'sandboxes':list(self.rows)}
        elif path.endswith('/files/list?depth=1'):
            value = {'entries':[] if self.fault == 'no-effect' else [{'path':controls.RUN_FILE}]}
        elif path == '/run':
            assert body is not None
            assert method == 'POST' and 'container' not in body and 'target' not in body
            assert body['env'] == {'ROUTE_RUN_MARKER':self.cfg['marker']}
            assert 'os.getuid()' in body['command'] and "'/proc/self/uid_map'" in body['command']
            self.path.mkdir(parents=True)
            self.alive = True
            self.rows = [{'id':'vm-1','name':'vm-1','persistent':False,'status':'Running','pid':123}]
            self.released.wait(.3)
            if self.fault != 'live-owner':
                self.alive = False
            if self.fault != 'listed-owner':
                self.rows = []
            if self.fault != 'path-leak':
                self.path.rmdir()
            if self.fault == 'failed-path':
                (self.root/'vm-1-failed-original').mkdir()
            value = {'exit_code':0,'truncated':self.fault == 'truncated',
                'stdout':{'encoding':'utf8','data':'wrong\n' if self.fault == 'wrong-response' else 'run-complete\n'},
                'stderr':{'encoding':'utf8','data':''}}
        else:
            raise AssertionError((method,path))
        return receipt(attempts=2 if self.fault == 'retry' and path == '/run' else 1),value
    def measure_request(self,method,path,**kwargs):
        if method == 'GET':
            body = b'0:0:0,0,4294967295\n'+controls.expected_marker('run',self.cfg)
            if self.fault == 'wrong-uid':
                body = b'1000:1000:0,0,4294967295\n'+controls.expected_marker('run',self.cfg)
            if self.fault == 'wrong-effect':
                body = b'0:0:0,0,4294967295\nforged\n'
            return receipt(body)
        assert method == 'POST' and path.endswith('?path='+controls.RELEASE)
        assert kwargs['body'] == b'release-original-run\n'
        self.released.set()
        return receipt(json.dumps({'success':True,'size':len(kwargs['body']),
            'vm_path':'/root/'+controls.RELEASE}).encode())


@pytest.mark.parametrize('fault',['','no-effect','wrong-uid','wrong-effect','live-owner',
    'listed-owner','path-leak','failed-path','truncated','wrong-response','retry'])
def test_original_vm_effect_fence_and_complete_ephemeral_settlement(monkeypatch,tmp_path,fault):
    cfg = {**controls.run_purge_config(),'poll_seconds':.001}
    backend = Backend(tmp_path,cfg,fault)
    cycle = controls.Cycle(backend,SimpleNamespace(label='service',headers={}),
        SimpleNamespace(tmp_dir=tmp_path),cfg,{})
    monkeypatch.setattr(cycle,'call',backend.call)
    monkeypatch.setattr(cycle,'session',lambda *args,**kwargs:backend.path)
    monkeypatch.setattr(subject,'Process',lambda pid:SimpleNamespace(is_running=lambda:backend.alive))
    if fault:
        with pytest.raises((AssertionError,ValueError)):
            subject.sample_run(cycle)
    else:
        subject.sample_run(cycle)
        assert not cycle.owned and len(cycle.metrics) == 4
    assert backend.calls.count(('POST','/run')) == 1


def test_empty_or_foreign_run_owner_never_becomes_independent_effect(tmp_path):
    cfg = controls.run_purge_config()
    completed = SimpleNamespace(done=lambda:True,result=lambda:None)
    cycle = SimpleNamespace(listing=lambda **kw:[],cfg=cfg)
    import time
    with pytest.raises(AssertionError,match='before independent'):
        subject.wait_original(cycle,completed,time.monotonic()+1)
    cycle.listing = lambda **kw:[{'id':'foreign','persistent':True}]
    with pytest.raises(AssertionError):
        subject.wait_original(cycle,completed,time.monotonic()+1)
