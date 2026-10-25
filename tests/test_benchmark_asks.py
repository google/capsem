"""Original waiter effect doubles; these source tests never launch a VM."""
from concurrent.futures import Future
from types import SimpleNamespace

import pytest

from benchmarks.helpers import route_asks as subject

GEN = '123e4567-e89b-42d3-a456-426614174000'
ASK = '123456789abc'


def listing():
    return {'vm_id':'vm','generation':GEN,'pending_count':1,'requests':[
        {'ask_id':ASK,'event_id':'event','status':'pending','rule_id':'corp.rules.'+subject.RULE}]}


@pytest.mark.parametrize('decision',['approve','deny'])
@pytest.mark.parametrize('fault',['none','early','wrong-rule','private','stale-accepted','duplicate-accepted','wrong-receipt','delivered-denial','replay','wrong-body','pending-remains'])
def test_same_original_decision_and_independent_effect(monkeypatch,decision,fault):
    pending = listing()
    original = Future()
    rows = []
    calls = []
    if fault == 'early':
        original.set_result(None)
    if fault == 'wrong-rule':
        pending['requests'][0]['rule_id'] = 'unrelated'
    if fault == 'private':
        pending['requests'][0]['summary'] = 'private-proof'
    def call(method,path,body=None,**kwargs):
        assert body is not None
        calls.append((method,path,body))
        if body['generation'] != GEN:
            assert kwargs['expected_status'] == 409
            return SimpleNamespace(status=200 if fault == 'stale-accepted' else 409),{}
        count = sum(value[2]['generation'] == GEN for value in calls)
        if count == 2:
            return SimpleNamespace(status=200 if fault == 'duplicate-accepted' else 409),{}
        status = 'approved' if decision == 'approve' else 'denied'
        result = {'status':200 if decision == 'approve' else 403,'body':'private-proof' if decision == 'approve' else 'denied'}
        if not original.done():
            original.set_result(result)
        if decision == 'approve' or fault == 'delivered-denial':
            rows.append({'method':'POST','path':'/content/echo','request_body':'private-proof', 'headers':{'host':subject.HOST}})
        if fault == 'wrong-body' and rows:
            rows[0]['request_body'] = 'wrong'
        if fault == 'replay' and rows:
            rows.append(rows[0])
        return SimpleNamespace(status=200),{'ask_id':'wrong' if fault == 'wrong-receipt' else ASK,'event_id':'event','status':status,'resolution_reason':'human_decision'}
    cycle = SimpleNamespace(cfg=subject.ask_config(),target=SimpleNamespace(label='service'),metrics={},call=call)
    def observe(operation,case,request,verify,**kwargs):
        receipt,value = request()
        assert receipt.status == 200
        return verify(value)
    cycle.observe_control = observe
    cycle.read = lambda *args,**kwargs:{'generation':GEN,'pending_count':1 if fault == 'pending-remains' else 0,'requests':[
        {**pending['requests'][0],'status':'approved' if decision == 'approve' else 'denied'}]}
    monkeypatch.setattr(subject,'original_result',lambda future,deadline:future.result(timeout=0))
    if fault == 'none' or (decision == 'deny' and fault in {'replay','wrong-body'}) or (decision == 'approve' and fault == 'delivered-denial'):
        subject.decide(cycle,'vm',decision,'private-proof',original,pending,lambda:rows,deadline=10**12)
        assert len(calls) == 3
    else:
        with pytest.raises(AssertionError):
            subject.decide(cycle,'vm',decision,'private-proof',original,pending,lambda:rows,deadline=10**12)


def test_exact_finite_config_and_counts():
    cfg = subject.ask_config()
    assert set(cfg['endpoints']) == {subject.OPERATION}
    assert cfg['endpoints'][subject.OPERATION]['cases'] == {'approve-original':'cycles','deny-original':'cycles'}
    with pytest.raises((ValueError,AssertionError)):
        subject.require_ask_metrics({},cfg,True)
