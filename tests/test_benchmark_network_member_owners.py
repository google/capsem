"""Existing strict transport/protocol/resource composition, with source doubles only."""
import time
from contextlib import contextmanager
from types import SimpleNamespace

import pytest

from benchmarks.helpers import route_network_member_probe as probe
from benchmarks.helpers import route_network_members as subject


def test_canonical_helpers_use_strict_single_attempt_adapter(monkeypatch):
    from tests.ironbank import test_managed_network_lifecycle as lifecycle
    from tests.ironbank.kingslanding import network
    calls = []
    response = {'exit_code':0,'truncated':False,'stdout':{'encoding':'utf8','data':'ok'},'stderr':{'encoding':'utf8','data':''}}
    cycle = SimpleNamespace(cfg=subject.member_config(),call=lambda *args,**kw:(calls.append((args,kw)) or (None,response)))
    monkeypatch.setattr(network,'cables',lambda bridge,vm:bridge.client().post('/vms/'+vm['id']+'/exec',{'target':'vm','command':'ip','timeout_secs':40},timeout=45))
    monkeypatch.setattr(network,'ping',lambda bridge,vm,address,count,**kw:(address,count,kw))
    monkeypatch.setattr(lifecycle,'switches',lambda service:[service])
    owner = probe.NetworkProbe(cycle,'service')
    assert owner.cables('owned') is response
    args,kw = calls[0]
    assert args[:2] == ('POST','/vms/owned/exec')
    assert args[2]['timeout_secs'] == cycle.cfg['exec_timeout_seconds']
    assert time.monotonic() < kw['deadline'] <= owner.deadline
    address,count,options = owner.ping('owned','10.20.0.2')
    assert address == '10.20.0.2' and count == cycle.cfg['probe_count']
    assert options == {'interval_ms':cycle.cfg['probe_interval_ms'],'timeout_ms':cycle.cfg['probe_timeout_ms']}
    assert owner.switches() == ['service']
    response['truncated'] = True
    with pytest.raises(AssertionError):
        owner.cables('owned')
    owner.deadline = time.monotonic()-1
    with pytest.raises(TimeoutError,match='deadline'):
        owner.cables('owned')


@pytest.mark.parametrize('broken',[False,True])
def test_finite_collector_retains_process_sampler_and_closes_transport(monkeypatch,broken):
    from benchmarks.helpers import route_lifecycle_fixture
    events = []
    @contextmanager
    def fixture(label):
        target = SimpleNamespace(transport_factory=lambda:SimpleNamespace(close=lambda:events.append(('close',label))))
        yield target,SimpleNamespace(proc=SimpleNamespace(pid=123)),'reference','certificate'
    class Cycle:
        def __init__(self,transport,target,service,cfg,metrics,probe_factory):
            pass
        def cleanup(self):
            events.append('cleanup')
        def listing(self):
            return []
    monkeypatch.setattr(route_lifecycle_fixture,'lifecycle_target',fixture)
    monkeypatch.setattr(subject,'Cycle',Cycle)
    monkeypatch.setattr(subject,'Process',lambda pid:('original-root',pid))
    def measure(metrics,key,root,operation):
        assert root == ('original-root',123)
        if broken:
            raise ValueError('partial observation')
        events.append(key)
        metrics[key+'.wall_s'] = {'unit':'seconds','samples':[1]}
    monkeypatch.setattr(subject,'measure',measure)
    monkeypatch.setattr(subject,'require_member_metrics',lambda metrics,cfg,quick:events.append('counts'))
    if broken:
        with pytest.raises(ValueError,match='partial'):
            subject.collect_network_members(True)
        assert events == [('close','service')]
    else:
        subject.collect_network_members(True)
        assert events == ['service.network_members.0','cleanup',('close','service'),
            'gateway.network_members.0','cleanup',('close','gateway'),'counts']
