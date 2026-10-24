"""Finite control effects with explicit source doubles, not VM measurements."""
import json
import shlex
from types import SimpleNamespace
from uuid import UUID

import pytest
from helpers.http_measurement import MeasuredResponse

from benchmarks.helpers import route_workload_controls as subject


class Backend:
    def __init__(self,failure=''):
        self.failure, self.calls, self.files = failure, [], {}
        self.pending, self.request_id = None, 0
        self.generation = '11111111-1111-4111-8111-111111111111'
        self.target = SimpleNamespace(label='service',headers={})

    def receipt(self,value,status=200):
        return MeasuredResponse(status,{},json.dumps(value).encode(),1,2,3,
            attempts=2 if self.failure == 'retry' else 1),value

    def call(self,method,path,body=None,*,deadline=None,expected_status=200):
        self.calls.append((method,path))
        if path.endswith('/browser/acknowledge'):
            assert isinstance(body,dict)
            if body['owner_generation'] != self.generation:
                assert UUID(body['owner_generation']).version == 4, 'wrong-epoch control must reach ownership conflict'
                return self.receipt({'error':'wrong generation'},409)
            if self.pending and (body['request_id'] == self.pending['id'] or self.failure == 'stale-clears-newer') and self.failure != 'ack-noop':
                self.pending = None
            return self.receipt({'vm_id':'live',**body})
        if '/files/list' in path:
            rows = [{'path':name,'name':name,'type':'file','size':len(data),'is_text':True,'mime':'text/plain'} for name,data in self.files.items()]
            return self.receipt({'entries':rows})
        if path == '/vms/list':
            return self.receipt({'sandboxes':[{'id':'live','browser_request':self.pending}]})
        return self.receipt({'id':'live','browser_request':self.pending})

    def execute(self,vm_id,command,*,deadline=None):
        if 'xdg-open' in command:
            self.request_id += 1
            url = shlex.split(command)[1]
            self.pending = {'id':self.request_id,'url':url,'owner_generation':self.generation}
        elif 'read_bytes' in command:
            assert self.files and self.failure != 'wrong-bytes', 'workload bytes differ'
        return self.receipt({'exit_code':0,'truncated':False,'stdout':[67,65,80,83,69,77], 'stderr':[]})


def test_browser_uses_real_original_metadata_then_replay_and_epoch_controls(monkeypatch):
    backend = Backend()
    cycle = subject.Cycle(None,backend.target,None,subject.controls_config(),{})
    monkeypatch.setattr(cycle,'call',backend.call)
    monkeypatch.setattr(cycle,'execute',backend.execute)
    monkeypatch.setattr(subject,'validate_exec',lambda *a:None)
    cycle.owned.add('live')
    subject.sample_browser(cycle,'live')
    assert backend.pending is None
    assert len(cycle.metrics) == 4
    assert sum(path.endswith('/browser/acknowledge') for _,path in backend.calls) == 4
    assert all('example.invalid' not in key and '00000000' not in key for key in cycle.metrics)


@pytest.mark.parametrize('failure',['ack-noop','stale-clears-newer','retry'])
def test_ack_echo_stale_clear_or_retry_cannot_pass(monkeypatch,failure):
    backend = Backend(failure)
    cycle = subject.Cycle(None,backend.target,None,subject.controls_config(),{})
    monkeypatch.setattr(cycle,'call',backend.call)
    monkeypatch.setattr(cycle,'execute',backend.execute)
    monkeypatch.setattr(subject,'validate_exec',lambda *a:None)
    cycle.owned.add('live')
    with pytest.raises((AssertionError,ValueError)):
        subject.sample_browser(cycle,'live')


def test_upload_preserves_original_binary_input_without_json_conversion(monkeypatch):
    cfg = subject.controls_config()
    calls = []
    def request(method,path,**kwargs):
        calls.append((method,path,kwargs))
        size = len(kwargs['body'])
        value = {'success':True,'size':size,'vm_path':'/root/route-upload-4096.txt','container_path':'/workspace/route-upload-4096.txt'}
        return MeasuredResponse(200,{},json.dumps(value).encode(),1,2,3)
    cycle = subject.Cycle(SimpleNamespace(measure_request=request),SimpleNamespace(label='service',headers={}),None,cfg,{})
    cycle.owned.add('live')
    data = subject.upload_bytes(4096)
    receipt,value = cycle.upload('live','route-upload-4096.txt',data,deadline=subject.time.monotonic()+5)
    assert calls[0][2]['body'] is data and receipt.attempts == 1
    assert value['size'] == 4096
    assert calls[0][2]['headers']['Content-Type'] == 'application/octet-stream'


def test_config_and_partial_sample_limits_are_explicit():
    from capsem_builder.gate.benchmarkcontrolschema import WorkloadControlConfig
    cfg = subject.controls_config()
    for change in ({'cycles':100},{'file_sizes':[]},{'file_sizes':[0]},{'file_sizes':[2097152]},{'endpoints':{}}):
        with pytest.raises(ValueError):
            WorkloadControlConfig.model_validate({**cfg,**change})
    with pytest.raises(ValueError,match='partial'):
        subject.require_control_metrics({},cfg,False)


def test_browser_independent_reads_allow_publication_race_but_not_stale_metadata(monkeypatch):
    cycle = subject.Cycle(None,SimpleNamespace(label='service'),None,subject.controls_config(),{})
    value = {'id':1,'owner_generation':str(UUID(int=17)),'url':'https://example.invalid/route-browser'}
    reads = iter([{'browser_request':None},{'browser_request':value}])
    monkeypatch.setattr(cycle,'info',lambda *a,**kw:next(reads))
    monkeypatch.setattr(cycle,'listing',lambda **kw:[{'id':'live','browser_request':value}])
    monkeypatch.setattr(subject.time,'sleep',lambda _:None)
    assert cycle.browser('live',deadline=subject.time.monotonic()+1) == value
    monkeypatch.setattr(cycle,'info',lambda *a,**kw:{'browser_request':None})
    with pytest.raises(TimeoutError,match='diverges'):
        cycle.browser('live',deadline=subject.time.monotonic()-1)
