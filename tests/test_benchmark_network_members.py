"""Explicit network effect doubles; no VM, router or packet generator runs."""
import json
from types import SimpleNamespace
from uuid import UUID

import pytest
from helpers.http_measurement import MeasuredResponse

from benchmarks.helpers import route_network_members as subject


class Backend:
    def __init__(self,failure=''):
        self.failure,self.calls,self.members,self.counter = failure,[],{},0
        self.retired = False
        self.created = False
        self.network = str(UUID(int=99))
        self.switch = SimpleNamespace(pid=123,create_time=lambda:1.,is_running=lambda:not self.retired)

    def measure_request(self,method,path,**kwargs):
        self.calls.append((method,path))
        data = json.loads(kwargs['body']) if kwargs.get('body') else {}
        status = 200
        if path == '/vms/create':
            self.counter += 1
            value = {'id':str(UUID(int=self.counter)), 'name':data['name'], 'status':'Running', 'available_actions':[]}
        elif path == '/networks' and method == 'POST':
            self.created = True
            status,value = 201,self.info()
        elif '/members/' in path:
            ident = path.rsplit('/',1)[1]
            if method == 'PUT' and self.failure != 'attach-noop':
                self.members[ident] = {'vm_id':ident,'state':'ready','address':f'10.20.0.{len(self.members)+1}','updated_unix_ms':1}
            elif method == 'DELETE' and self.failure != 'detach-noop':
                del self.members[ident]
            value = self.info()
        elif '/logs' in path:
            value = subject_test_logs(self)
        elif method == 'DELETE':
            assert not self.members, 'retirement requires an empty network'
            if self.failure != 'retire-noop':
                self.retired = True
            value = {'success':True}
        else:
            value = self.info()
        return MeasuredResponse(status,{},json.dumps(value).encode(),1,2,3,attempts=2 if self.failure == 'retry' else 1)

    def info(self):
        return {'id':self.network,'name':subject.member_config()['name'],'subnet':'10.20.0.0/24','created_unix_ms':1,'members':list(self.members.values())}


class Probe:
    def __init__(self,backend):
        self.backend,self.deadline = backend,0.

    def cables(self,vm_id):
        rows = [row for ident,row in self.backend.members.items() if ident == vm_id]
        if self.backend.failure == 'missing-cable':
            return {}
        return {f'cable{index}':row['address']+'/24' for index,row in enumerate(rows)}

    def ping(self,source,address):
        cfg = subject.member_config()
        live = address in {row['address'] for row in self.backend.members.values()}
        received = cfg['probe_count'] if live else 0
        if self.backend.failure == 'no-reachability':
            received = 0
        return {'sent':cfg['probe_count'],'received':received,'lost':cfg['probe_count']-received}

    def switches(self):
        if self.backend.failure == 'switch-replaced' and len(self.backend.members) == 2:
            return [SimpleNamespace(pid=999,create_time=lambda:1.)]
        return [self.backend.switch] if self.backend.created and not self.backend.retired else []


def subject_test_logs(backend):
    events: list[dict] = []
    for index,(ident,address,kind) in enumerate([
        (str(UUID(int=1)),'10.20.0.1','network.connect'),
        (str(UUID(int=2)),'10.20.0.2','network.connect'),
        (str(UUID(int=2)),'10.20.0.2','network.close'),
    ],1):
        endpoint = {'vm':{'id':ident},'address':address,'port':0}
        event = {'network':{'context':'private','protocol':'link','source':endpoint,'destination':endpoint},
                 'decision':{'effective':'allow','reason':'linked' if kind.endswith('connect') else 'unlinked'}}
        if kind.endswith('close'):
            event['frames'] = {'reason':'Cancelled','frames_in':2,'frames_out':2,'bytes_in':128,'bytes_out':128,
                'dropped':dict.fromkeys(['short','source_mac','source_address','unknown','queue_full','storm'],0)}
        events.append({'sequence':index,'event_id':f'{index:012x}','timestamp_unix_ms':1,'event_type':kind,
                       'connection_id':str(UUID(int=10 if index == 1 else 11)),'event':event})
    if backend.failure == 'fake-flow':
        events[0]['event']['network']['protocol'] = 'tcp'
    return {'events':events,'cursor':'opaque'}


def setup(monkeypatch,failure=''):
    backend = Backend(failure)
    cycle = subject.Cycle(backend,SimpleNamespace(label='service',headers={}),None,subject.member_config(),{},lambda _:Probe(backend))
    cycle.cfg = {**cycle.cfg,'timeout_seconds':0.2,'poll_seconds':0.001}
    monkeypatch.setattr(cycle,'listing',lambda **kw:[])
    monkeypatch.setattr(cycle,'ready',lambda *a,**kw:None)
    return cycle,backend


def test_real_effect_sequence_is_distinct_from_reply_and_flow_logs(monkeypatch,tmp_path):
    cycle,backend = setup(monkeypatch)
    cert = tmp_path/'cert'
    cert.write_text('synthetic certificate')
    subject.sample_members(cycle,'reference',cert)
    assert len(cycle.metrics) == 12 and backend.members == {} and backend.retired
    assert sum(method == 'PUT' for method,_ in backend.calls) == 2
    assert sum(method == 'DELETE' and '/members/' in path for method,path in backend.calls) == 2
    subject.require_member_metrics({**cycle.metrics,'service.network_members.0.wall_s':{'samples':[1]}},cycle.cfg,True,labels=('service',))


@pytest.mark.parametrize('failure',['attach-noop','detach-noop','missing-cable','no-reachability','fake-flow','retry','switch-replaced','retire-noop'])
def test_noop_broken_cable_packets_or_fabricated_flow_cannot_pass(monkeypatch,tmp_path,failure):
    cycle,_ = setup(monkeypatch,failure)
    cert = tmp_path/'cert'
    cert.write_text('synthetic certificate')
    with pytest.raises((AssertionError,ValueError,TimeoutError)):
        subject.sample_members(cycle,'reference',cert)


def test_partial_duplicate_samples_and_budget_changes_fail(monkeypatch,tmp_path):
    from capsem_builder.gate.benchmarkmemberschema import NetworkMemberConfig
    cfg = subject.member_config()
    for change in ({'cycles':5},{'quick_cycles':3},{'timeout_seconds':1},{'probe_count':100},{'log_limit':3},{'endpoints':{}}):
        with pytest.raises(ValueError):
            NetworkMemberConfig.model_validate({**cfg,**change})
    with pytest.raises(ValueError,match='partial'):
        subject.require_member_metrics({},cfg,True)
    cycle,_ = setup(monkeypatch)
    cert = tmp_path/'cert'
    cert.write_text('synthetic')
    subject.sample_members(cycle,'reference',cert)
    cycle.metrics['service.network_members.0.wall_s'] = {'samples':[1]}
    key = next(iter(cycle.metrics))
    cycle.metrics[key]['samples'] *= 2
    with pytest.raises(ValueError,match='partial'):
        subject.require_member_metrics(cycle.metrics,cfg,True,labels=('service',))


@pytest.mark.parametrize('fault',['missing','duplicate','connection','order','frames','spoof','cursor','wrong-address'])
def test_log_integrity_requires_exact_original_connection_and_packet_facts(fault):
    backend = Backend()
    value = subject_test_logs(backend)
    if fault == 'missing':
        value['events'].pop()
    elif fault == 'duplicate':
        value['events'][2] = value['events'][1]
    elif fault == 'connection':
        value['events'][2]['connection_id'] = str(UUID(int=55))
    elif fault == 'order':
        value['events'][2]['sequence'] = 0
    elif fault == 'frames':
        value['events'][2]['event']['frames']['frames_in'] = 0
    elif fault == 'spoof':
        value['events'][2]['event']['frames']['dropped']['source_address'] = 1
    elif fault == 'cursor':
        value['next_cursor'] = 'more'
    else:
        value['events'][0]['event']['network']['source']['address'] = '10.0.0.99'
    with pytest.raises(AssertionError):
        subject.validate_link_logs(value,{str(UUID(int=1)):'10.20.0.1',str(UUID(int=2)):'10.20.0.2'},str(UUID(int=2)),16)
