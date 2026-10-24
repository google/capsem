"""Five original preview operations against one genuine disposable owner."""
import time

from .route_measurements import validate_receipt
from .route_preview import (
    CREATE,
    DELETE,
    MINT,
    REVOKE,
    SURFACE,
    Preview,
    bootstrap_cookie,
    material,
    refused,
    validate_owners,
)
from .route_preview_fixture import start_workload


def mint(cycle,gateway,operation,path,verify,deadline):
    # Mint belongs exclusively to TCP gateway. UDS fixture uses the same
    # gateway untimed for setup/effects, never counts its 404 as success.
    if cycle.target.label == 'gateway':
        (case,) = cycle.cfg['endpoints'][operation]['cases']
        return cycle.observe_control(operation,case,
            lambda:gateway.call('POST',path,deadline=deadline),verify,deadline=deadline)
    receipt,value = gateway.call('POST',path,deadline=deadline)
    validate_receipt(receipt)
    return verify(value)


def exchange(preview,value,*,surface):
    before = preview.hits()
    path,body = material(value,preview.exposure,preview.port,preview.cycle.cfg)
    cookie = bootstrap_cookie(preview.request('POST',path,body=body,form=True),surface)
    replay = preview.request('POST',path,body=body,form=True)
    assert replay.status == 400, 'original bootstrap replay was not rejected'
    assert preview.hits() == before, 'bootstrap reached guest workload'
    return cookie


def surface_effect(preview,cookie):
    page = preview.request('GET','/_capsem/surface/',cookie=cookie)
    validate_receipt(page)
    assert page.status == 200 and page.headers['content-type'] == 'text/html; charset=utf-8'
    origin = f'{preview.exposure}.localhost:{preview.port}'
    assert f"connect-src 'self' ws://{origin}" in page.headers['content-security-policy']
    assert b'<script src="capsem-surface.js"></script>' in page.body
    assert b'<script src="xpra-main.js"></script>' in page.body and b'<script>' not in page.body
    reply = preview.request('GET','/hello?from=kingslanding',cookie=cookie)
    validate_receipt(reply)
    assert reply.status == 200 and reply.body == b'workload:GET /hello?from=kingslanding HTTP/1.1', 'declared port did not deliver original responder bytes'


def delivered(preview,cookie):
    before = preview.hits()
    reply = preview.request('GET','/final',cookie=cookie)
    validate_receipt(reply)
    assert reply.status == 200 and reply.body == b'redirected'
    assert preview.hits() == before+1, 'original preview request missing/duplicated'


def sample_preview(cycle,gateway,reference,certificate):
    cfg = cycle.cfg
    deadline = time.monotonic()+cfg['timeout_seconds']
    assert cycle.listing(deadline=deadline) == []
    _,value = cycle.call('POST','/vms/create',{'name':'route-preview-controls','persistent':True,
        'cpus':cfg['cpus'],'ram_mb':cfg['ram_mb'],'container':{'image':reference,
        'registry':{'ca_pem':certificate.read_text()}}},deadline=deadline)
    vm_id = cycle.identify(value)
    cycle.ready(vm_id,reference,deadline=deadline)
    base = f'/vms/{vm_id}'
    # The owner publishes its actual declared surface after workload launch.
    while True:
        container = cycle.read(base+'/container',deadline=deadline)
        surface = container.get('surface') or {}
        if surface.get('exposure_id'):
            break
        if time.monotonic() >= deadline:
            raise TimeoutError('declared surface exposure was not published')
        time.sleep(cfg['poll_seconds'])
    a = {'id':surface['exposure_id'],'host_port':None,'guest_port':14500,'target':'container','access':'http_preview'}
    original = cycle.read(base+'/exposures',deadline=deadline)
    generation = original['owner_generation']
    validate_owners(original,container,generation,a)
    port = int((cycle.service.tmp_dir/'preview.port').read_text())
    assert 0 < port <= 65535
    hits = start_workload(cycle,vm_id,deadline)
    a_preview = Preview(cycle,a['id'],port,hits,deadline=deadline)
    b_preview = None
    try:
        def surfaced(value):
            cookie = exchange(a_preview,value,surface=True)
            surface_effect(a_preview,cookie)
            return cookie
        a_cookie = mint(cycle,gateway,SURFACE,base+'/surface/session',surfaced,deadline)
        def owners(b=None):
            validate_owners(cycle.read(base+'/exposures',deadline=deadline),
                cycle.read(base+'/container',deadline=deadline),generation,a,b)
        def declared(value):
            assert set(value) == set(a) and value == {**a,'id':value['id'],'guest_port':8080}
            assert value['id'] != a['id']
            owners(value)
            return value
        b = cycle.observe_control(CREATE,'owned-declaration',
            lambda:cycle.call('POST',base+'/exposures',{'target':'container','guest_port':8080,'host_port':0,'access':'http_preview'},deadline=deadline),
            declared,deadline=deadline)
        b_preview = Preview(cycle,b['id'],port,hits,deadline=deadline)
        path = base+'/exposures/'+b['id']
        def opened(value):
            cookie = exchange(b_preview,value,surface=False)
            delivered(b_preview,cookie)
            return cookie
        b_cookie = mint(cycle,gateway,MINT,path+'/preview-session',opened,deadline)
        before = hits()
        refused(b_preview,None,before=before,status=401)
        refused(b_preview,a_cookie,before=before,status=403)
        # Reopen the original B flow after refusal controls closed its socket.
        delivered(b_preview,b_cookie)
        def revoked(value):
            assert value == {'revoked':1}, 'original preview session not retired exactly once'
            b_preview.retired()
            owners(b)
            refused(b_preview,b_cookie,before=hits(),status=403)
        cycle.observe_control(REVOKE,'original-flow-revoked',
            lambda:cycle.call('DELETE',path+'/preview-session',deadline=deadline),revoked,deadline=deadline)
        # Independent genuine new capability; never replay the original revoke.
        _,fresh = gateway.call('POST',path+'/preview-session',deadline=deadline)
        fresh_cookie = opened(fresh)
        def deleted(value):
            assert value == {'success':True}
            b_preview.retired()
            owners()
            refused(b_preview,fresh_cookie,before=hits(),status=403)
            _,error = gateway.call('POST',path+'/preview-session',deadline=deadline,expected_status=404)
            assert error, 'removed original exposure unexpectedly minted a session'
            surface_effect(a_preview,a_cookie)
        cycle.observe_control(DELETE,'original-flow-deleted',
            lambda:cycle.call('DELETE',path,deadline=deadline),deleted,deadline=deadline)
    finally:
        a_preview.close()
        if b_preview is not None:
            b_preview.close()
