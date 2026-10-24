"""Explicit error-only source doubles; no enrollment or runtime launched."""
import json
from types import SimpleNamespace

import pytest
from helpers.http_measurement import MeasuredResponse

from benchmarks.helpers import route_connection_errors as subject


def test_exact_typed_refusals_are_not_generic_failure_or_success():
    cfg = subject.error_config()
    for entry in cfg['endpoints'].values():
        receipt = MeasuredResponse(entry['status'],{'content-type':'application/json'},
            json.dumps({'error':'credential request refused','code':entry['code']}).encode(),1,2,3)
        subject.validate_error(receipt,entry)
        for bad in (
            MeasuredResponse(401,receipt.headers,receipt.body,1,2,3),
            MeasuredResponse(404,{},b'not found',1,2,3),
            MeasuredResponse(200,receipt.headers,receipt.body,1,2,3),
            MeasuredResponse(entry['status'],receipt.headers,b'{"error":"not found"}',1,2,3),
            MeasuredResponse(entry['status'],receipt.headers,receipt.body,1,2,3,attempts=2),
        ):
            with pytest.raises((AssertionError,ValueError)):
                subject.validate_error(bad,entry)


def test_exact_seven_error_cases_keep_method_and_public_identity_inputs():
    cfg = subject.error_config()
    cases = subject.requests(cfg)
    assert len(cases) == len(cfg['endpoints']) == 7
    assert sum(body is not None for _,_,body in cases.values()) == 2
    for operation,(method,path,body) in cases.items():
        assert operation.startswith(method+' ') and '{' not in path
        if body is not None:
            assert body['request_id'] == cfg['request_id']
    assert cfg['success_coverage'] is False


def test_all_samples_have_explicit_error_namespace_and_no_opaque_id(monkeypatch):
    cfg = subject.error_config()
    target = SimpleNamespace(label='service',headers={})
    expected = {path:cfg['endpoints'][operation] for operation,(_,path,_) in subject.requests(cfg).items()}
    calls = []
    def request(method,path,**kwargs):
        calls.append((method,path))
        entry = expected[path]
        # GET/DELETE share identity/error contract for these existing routes.
        return MeasuredResponse(entry['status'],{'content-type':'application/json'},
            json.dumps({'error':'credential request refused','code':entry['code']}).encode(),1,2,3)
    metrics = subject.sample_errors(SimpleNamespace(measure_request=request),target,cfg,quick=True)
    assert len(metrics) == 21 and len(calls) == 7*cfg['quick_samples']
    assert all(key.startswith('service.error_path.') for key in metrics)
    assert all(cfg['transaction_id'] not in key and cfg['connection_id'] not in key for key in metrics)
    assert all(cfg['grant_id'] not in key and cfg['request_id'] not in key and subject.VM_ID not in key for key in metrics)
    metrics['service.connection_error_paths.wall_s'] = {'unit':'seconds','samples':[1]}
    subject.require_error_metrics(metrics,cfg,True,labels=('service',))
    metrics[next(iter(metrics))]['samples'].pop()
    with pytest.raises(ValueError,match='partial'):
        subject.require_error_metrics(metrics,cfg,True,labels=('service',))


def test_error_config_cannot_certify_authority_auth_or_malformed_input():
    from capsem_builder.gate.benchmarkerrorschema import ConnectionErrorConfig
    cfg = subject.error_config()
    for change in ({'success_coverage':True},{'samples':1000},{'transaction_id':'bad'},{'endpoints':{}}):
        with pytest.raises(ValueError):
            ConnectionErrorConfig.model_validate({**cfg,**change})
    endpoints = {**cfg['endpoints']}
    key = next(iter(endpoints))
    endpoints[key] = {**endpoints[key],'status':401}
    with pytest.raises(ValueError):
        ConnectionErrorConfig.model_validate({**cfg,'endpoints':endpoints})
