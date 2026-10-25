"""Original HTTP waiter decision; resolver receipt excludes pending/human dwell."""
import json
import shlex
import time
import tomllib
from concurrent.futures import ThreadPoolExecutor
from functools import partial
from pathlib import Path
from types import SimpleNamespace
from uuid import UUID, uuid4

from helpers.service import exec_output_text
from ironbank.approval_fixture import pending_original_operation
from psutil import Process

from .route_ask_fixture import HOST, RULE, ask_target
from .route_credentials import require_credential_metrics
from .route_workload_controls import Cycle
from .stream_measurements import measure, sample_count

OPERATION = 'POST /vms/{id}/security/asks/{ask_id}/resolve'


def ask_config():
    from capsem_builder.gate.benchmarkaskschema import AskRouteConfig
    raw = tomllib.loads((Path(__file__).resolve().parents[2]/'config/gate.toml').read_text())
    return AskRouteConfig.model_validate(raw['benchmark']['routes']['ask_controls']).model_dump()


def original_result(original,deadline):
    _,value = original.result(timeout=max(0,deadline-time.monotonic()))
    assert value['exit_code'] == 0 and value['truncated'] is False
    assert exec_output_text(value,'stderr') == ''
    mapping,separator,text = exec_output_text(value).partition('\n')
    assert separator and mapping.startswith('CAPSEM-UID-MAP ')
    assert mapping.removeprefix('CAPSEM-UID-MAP ').split() != ['0','0','4294967295']
    result = json.loads(text)
    assert set(result) == {'status','body'}
    return result


def raw(cycle,case,name,milliseconds):
    assert milliseconds >= 0
    key = f'{cycle.target.label}.{OPERATION} [{case}].{name}'
    cycle.metrics.setdefault(key,{'unit':'milliseconds','samples':[]})['samples'].append(milliseconds)


def decide(cycle,vm_id,decision,marker,original,listing,records,*,deadline):
    assert not original.done(), 'original returned before decision'
    assert listing['vm_id'] == vm_id and listing['pending_count'] == 1
    generation = listing['generation']
    assert str(UUID(generation)) == generation and UUID(generation).int != 0
    (ask,) = [row for row in listing['requests'] if row['status'] == 'pending']
    assert len(ask['ask_id']) == 12 and all(char in '0123456789abcdef' for char in ask['ask_id'])
    assert ask['rule_id'] == 'corp.rules.'+RULE
    assert marker not in json.dumps(listing), 'review projection exposed original body'
    assert records() == [], 'unresolved original reached upstream'
    route = f'/vms/{vm_id}/security/asks'
    resolve = f"{route}/{ask['ask_id']}/resolve"
    body = {'generation':generation,'decision':decision}
    receipt,_ = cycle.call('POST',resolve,{**body,'generation':str(uuid4())},deadline=deadline,expected_status=409)
    assert receipt.status == 409 and not original.done() and records() == []
    expected = 'approved' if decision == 'approve' else 'denied'
    def verify(value):
        assert value['ask_id'] == ask['ask_id'] and value['event_id'] == ask['event_id']
        assert value['status'] == expected and value['resolution_reason'] == 'human_decision'
    case = decision+'-original'
    started = time.monotonic()
    cycle.observe_control(OPERATION,case,
        lambda:cycle.call('POST',resolve,body,deadline=deadline),verify,deadline=deadline)
    result = original_result(original,deadline)
    assert result['status'] == (200 if decision == 'approve' else 403)
    if decision == 'approve':
        assert result['body'] == marker
        while not records():
            assert time.monotonic() < deadline, 'approved original missing upstream'
            time.sleep(cycle.cfg['poll_seconds'])
        delivered = records()
        assert len(delivered) == 1, 'original HTTP request replayed'
        (row,) = delivered
        assert row['method'] == 'POST' and row['path'] == '/content/echo'
        assert row['headers']['host'] == HOST and row['request_body'] == marker
    else:
        assert marker not in result['body'] and records() == []
    raw(cycle,case,'original_completion_after_decision_ms',(time.monotonic()-started)*1000)
    receipt,_ = cycle.call('POST',resolve,body,deadline=deadline,expected_status=409)
    assert receipt.status == 409
    final = cycle.read(route,deadline=deadline)
    assert final['generation'] == generation and final['pending_count'] == 0
    (row,) = [row for row in final['requests'] if row['ask_id'] == ask['ask_id']]
    assert row['status'] == expected and row['event_id'] == ask['event_id']
    assert len(records()) == (1 if decision == 'approve' else 0), 'duplicate replay reached upstream'


def sample_ask(cycle,reference,certificate,records,decision):
    cfg,deadline = cycle.cfg,time.monotonic()+cycle.cfg['timeout_seconds']
    assert cycle.listing(deadline=deadline) == []
    _,created = cycle.call('POST','/vms/create',{'persistent':False,'cpus':cfg['cpus'],'ram_mb':cfg['ram_mb'],
        'container':{'image':reference,'registry':{'ca_pem':certificate.read_text()}}},deadline=deadline)
    vm_id = cycle.identify(created)
    cycle.ready(vm_id,reference,deadline=deadline,persistent=False)
    marker = 'capsem-original-ask-'+uuid4().hex
    source = f'''import json,urllib.request,urllib.error
request=urllib.request.Request('https://{HOST}/content/echo',data={marker!r}.encode(),headers={{'content-type':'text/plain'}})
try:
    with urllib.request.urlopen(request,timeout={cfg['exec_timeout_seconds']}) as response:
        value={{'status':response.status,'body':response.read().decode()}}
except urllib.error.HTTPError as response:
    value={{'status':response.code,'body':response.read().decode()}}
print(json.dumps(value))
'''
    def original_operation():
        # A distinct instance owns only the original caller's socket. Closing
        # a shared Transport closes every thread's sockets, including resolver.
        transport = cycle.target.transport_factory()
        caller = Cycle(transport,cycle.target,cycle.service,cfg,{})
        caller.owned.add(vm_id)
        try:
            return caller.execute(vm_id,'python3 -c '+shlex.quote(source),deadline=deadline)
        finally:
            transport.close()
    with ThreadPoolExecutor(max_workers=1) as executor:
        started = time.monotonic()
        original = executor.submit(original_operation)
        def listing(method,path,*,timeout):
            receipt,value = cycle.call(method,path,deadline=min(deadline,time.monotonic()+timeout))
            return receipt.status,value
        pending = pending_original_operation(SimpleNamespace(call_json=listing),vm_id,original,
            timeout=min(cfg['exec_timeout_seconds'],max(0,deadline-time.monotonic())))
        raw(cycle,decision+'-original','original_to_pending_ms',(time.monotonic()-started)*1000)
        decide(cycle,vm_id,decision,marker,original,pending,records,deadline=deadline)
    # Existing canonical approval Ironbank owns physical audit after stop/drain.
    cycle.cleanup()
    assert cycle.listing(deadline=deadline) == []


def require_ask_metrics(metrics,cfg,quick,labels=('service','gateway')):
    for decision in ('approve','deny'):
        require_credential_metrics(metrics,cfg,quick,labels,resource_domain='ask_controls.'+decision)
    for label in labels:
        for case in ('approve-original','deny-original'):
            for name in ('original_to_pending_ms','original_completion_after_decision_ms'):
                values = metrics[f'{label}.{OPERATION} [{case}].{name}']
                assert values['unit'] == 'milliseconds' and len(values['samples']) == sample_count(cfg,'cycles',quick)


def collect_ask_controls(quick):
    cfg,metrics = ask_config(),{}
    for label in ('service','gateway'):
        for index in range(sample_count(cfg,'cycles',quick)):
            for decision in ('approve','deny'):
                with ask_target(label,cfg) as (target,service,reference,certificate,records):
                    transport = target.transport_factory()
                    cycle = Cycle(transport,target,service,cfg,metrics)
                    try:
                        assert service.proc is not None
                        measure(metrics,f'{label}.ask_controls.{decision}.{index}',Process(service.proc.pid),
                            partial(sample_ask,cycle,reference,certificate,records,decision))
                    finally:
                        transport.close()
    require_ask_metrics(metrics,cfg,quick)
    return metrics
