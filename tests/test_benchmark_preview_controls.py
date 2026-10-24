"""Preview controls are genuine effect checks; these doubles launch no VM."""
import copy
from types import SimpleNamespace

import pytest
from helpers.http_measurement import MeasuredResponse

from benchmarks.helpers import route_preview as subject


def receipt(status=200,body=b'body',headers=None):
    return MeasuredResponse(status,headers or {},body,1,None if not body else 2,3,attempts=1)


def test_bounded_bootstrap_material_is_original_single_use_and_exact_origin():
    cfg = subject.preview_config()
    value = {'url':'http://original.localhost:1234/_capsem/bootstrap',
        'bootstrap_token':'synthetic-original','expires_in_seconds':30}
    assert subject.material(value,'original',1234,cfg) == ('/_capsem/bootstrap',b'bootstrap_token=synthetic-original')
    for changed in ({'url':value['url']+'?token=secret'}, {'url':value['url'].replace('original','foreign')},
            {'url':value['url'].replace(':1234',':1235')},{'bootstrap_token':''},{'expires_in_seconds':0},
            {'expires_in_seconds':31},{'bootstrap_token':'x'*(cfg['max_token_bytes']+1)}):
        with pytest.raises((AssertionError,ValueError)):
            subject.material({**value,**changed},'original',1234,cfg)


def test_empty_bootstrap_has_no_invented_first_body_sample_and_safe_cookie():
    headers = {'location':'/','set-cookie':'capsem_preview=opaque; Path=/; HttpOnly; SameSite=Lax; Max-Age=900','cache-control':'no-store'}
    assert subject.bootstrap_cookie(receipt(303,b'',headers),False) == 'capsem_preview=opaque'
    for changed in ({'location':'https://foreign/'},{'set-cookie':'capsem_preview=opaque; Path=/'},{'cache-control':'public'}):
        with pytest.raises(AssertionError):
            subject.bootstrap_cookie(receipt(303,b'',{**headers,**changed}),False)
    with pytest.raises(AssertionError):
        subject.bootstrap_cookie(receipt(200,b'',headers),False)
    with pytest.raises(AssertionError):
        subject.bootstrap_cookie(receipt(303,b'ignored',headers),False)


def test_exposure_owner_and_declared_surface_cannot_be_replaced_or_deleted():
    a = {'id':'surface-a','host_port':None,'guest_port':14500,'target':'container','access':'http_preview'}
    b = {**a,'id':'manual-b','guest_port':8080}
    listing = {'owner_generation':'18446744073709551615','exposures':[a,b]}
    surface = {'state':'running','surface':{'kind':'xpra','port':14500,'exposure_id':'surface-a'}}
    subject.validate_owners(listing,surface,listing['owner_generation'],a,b)
    for fault in ('generation','surface','missing','duplicate','changed'):
        rows,container = copy.deepcopy(listing),copy.deepcopy(surface)
        assert isinstance(container['surface'],dict) and isinstance(rows['exposures'],list)
        if fault == 'generation':
            rows['owner_generation'] = 'foreign'
        if fault == 'surface':
            container['surface']['exposure_id'] = 'manual-b'
        if fault == 'missing':
            rows['exposures'].pop()
        if fault == 'duplicate':
            rows['exposures'].append(b)
        if fault == 'changed':
            rows['exposures'][1]['guest_port'] = 99
        with pytest.raises(AssertionError):
            subject.validate_owners(rows,container,listing['owner_generation'],a,b)


@pytest.mark.parametrize('value',['1','18446744073709551615'])
def test_original_publication_generation_preserves_all_u64_bits(value):
    assert subject.publication_generation(value) == value


@pytest.mark.parametrize('value',['','0','01','+1','-1','18446744073709551616','123e4567-e89b-42d3-a456-426614174000','\u0661',1,None])
def test_publication_generation_rejects_noncanonical_or_foreign_identity(value):
    with pytest.raises(AssertionError):
        subject.publication_generation(value)


def test_refusal_requires_exact_status_no_guest_hit_and_original_close():
    calls = []
    class Peer:
        count = 2
        def close(self):
            pass
        def request(self,*args,**kwargs):
            calls.append('request')
            return receipt(403,b'refused')
        def hits(self):
            return self.count
    peer = Peer()
    subject.refused(peer,'opaque',before=2,status=403)
    peer.count = 3
    with pytest.raises(AssertionError):
        subject.refused(peer,'opaque',before=2,status=403)
    assert calls == ['request','request']


def test_schema_pins_gateway_mint_and_partial_metrics_fail():
    from capsem_builder.gate.benchmarkpreviewschema import PreviewRouteConfig
    cfg = subject.preview_config()
    for operation,entry in cfg['endpoints'].items():
        assert entry['target_scope'] == ('gateway' if operation in {subject.MINT,subject.SURFACE} else 'both')
    for changed in ({'cycles':5},{'quick_cycles':3},{'timeout_seconds':901},{'bootstrap_lifetime_seconds':31}):
        with pytest.raises(ValueError):
            PreviewRouteConfig.model_validate({**cfg,**changed})
    wrong = copy.deepcopy(cfg)
    wrong['endpoints'][subject.MINT]['target_scope'] = 'both'
    with pytest.raises(ValueError):
        PreviewRouteConfig.model_validate(wrong)
    with pytest.raises(ValueError,match='partial'):
        subject.require_preview_metrics({},cfg,True)


def test_preview_transport_never_receives_gateway_bearer(monkeypatch):
    captured = []
    class Transport:
        def __init__(self,**kwargs):
            captured.append(kwargs)
        def measure_request(self,*args,**kwargs):
            captured.append(kwargs)
            return receipt()
        def close(self):
            captured.append('close')
    monkeypatch.setattr(subject,'Transport',Transport)
    cycle = SimpleNamespace(cfg=subject.preview_config())
    probe = subject.Preview(cycle,'owned',1234,lambda:0)
    try:
        probe.request('GET','/final',cookie='capsem_preview=opaque')
    finally:
        probe.close()
    assert captured[0] == {'host':'127.0.0.1','port':1234}
    headers = captured[1]['headers']
    assert headers == {'Host':'owned.localhost:1234','Cookie':'capsem_preview=opaque'}
    assert captured[-1] == 'close'
