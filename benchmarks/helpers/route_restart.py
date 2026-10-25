"""One-shot native supervisor restart, never ordinary confined route setup."""
import json
import sys
import time
import tomllib
from contextlib import contextmanager
from functools import partial
from pathlib import Path

from helpers.constants import ASSETS_DIR
from helpers.http_transport import Transport
from helpers.managed_service import ReadinessPolicy, managed_service
from helpers.service import ServiceInstance
from helpers.stopped_workspace import VM_ID, seed_stopped_workspace

from .route_measurements import Observations, validate_receipt
from .route_requests import Target, json_request
from .stream_measurements import measure

OPERATION = 'POST /restart'
CLOCKS = ('original_retired_ms','new_ready_ms','reconnected_ms','verified_completion_ms')
PROBES = ('readiness_headers_ms','readiness_first_body_byte_ms','readiness_completion_ms')


def restart_config():
    from capsem_builder.gate.benchmarkrestartschema import RestartRouteConfig
    raw = tomllib.loads((Path(__file__).resolve().parents[2]/'config/gate.toml').read_text())
    return RestartRouteConfig.model_validate(raw['benchmark']['routes']['restart_controls']).model_dump()


def manager_name():
    if sys.platform not in {'darwin','linux'}:
        raise ValueError('native restart requires an existing supported supervisor')
    return 'launchd' if sys.platform == 'darwin' else 'systemd'


def policy(cfg,deadline):
    return ReadinessPolicy(deadline,cfg['poll_seconds'],cfg['max_response_bytes'],cfg['max_attempts'])


def bounded_read(path,maximum):
    assert not path.is_symlink() and path.is_file(), 'restart artifact not original regular file'
    with path.open('rb') as source:
        raw = source.read(maximum+1)
    assert len(raw) <= maximum, 'restart artifact exceeds configured budget'
    return raw


def workspace_expected(cfg):
    return {**{f'unchanged-{index:03}.txt':b'unchanged\n'*32 for index in range(cfg['workspace_files'])},
        'modified.txt':b'after\n','created.txt':b'created\n'}


def preserved_files(service,cfg):
    root = service.tmp_dir/'persistent'/VM_ID/'guest/workspace'
    assert not any(path.is_symlink() for path in (service.tmp_dir/'persistent',root.parent.parent,root.parent,root))
    expected = workspace_expected(cfg)
    assert {path.name for path in root.iterdir()} == set(expected), 'stopped workspace membership changed'
    assert sum(map(len,expected.values())) <= cfg['max_workspace_bytes']
    for name,data in expected.items():
        assert bounded_read(root/name,len(data)) == data, 'stopped workspace bytes changed'
    return json.loads(bounded_read(service.tmp_dir/'persistent_registry.json',cfg['max_registry_bytes']))


def target_for(label,service,identity,*,token=None):
    if label == 'service':
        return Target(label,lambda:Transport(socket_path=str(service.uds_path)),{}, {})
    assert label == 'gateway'
    return Target(label,lambda:Transport(host='127.0.0.1',port=identity.port),
        {'Authorization':'Bearer '+(identity.token if token is None else token)}, {})


def call(target,method,path,cfg,deadline,*,status=200,body=None):
    remaining = deadline-time.monotonic()
    assert remaining > 0, 'restart total deadline exceeded'
    transport = target.transport_factory()
    try:
        result = json_request(transport,target,method,path,body,timeout=remaining,
            maximum=cfg['max_response_bytes'],expected_status=status)
    finally:
        transport.close()
    assert time.monotonic() <= deadline, 'restart completed after deadline'
    return result


def sample_restart(owner,service,before,inventory,cfg,label):
    registry = preserved_files(service,cfg)
    original = owner.owned_roots()
    assert original, 'original owned roots absent'
    deadline = time.monotonic()+cfg['timeout_seconds']
    started = time.monotonic_ns()
    receipt,value = call(target_for(label,service,before),'POST','/restart',cfg,deadline,status=202)
    assert value == {'status':'accepted','manager':manager_name(),'authentication':'new_token_required'}, 'restart admission contract differs'
    observed_at = time.monotonic_ns()
    observed = owner.ready_observed(before,policy=policy(cfg,deadline),original=original)
    after = observed.identity
    assert after.service_pid != before.service_pid and after.gateway_pid != before.gateway_pid, 'original supervisor identity did not retire'
    assert after.token and after.token != before.token, 'gateway token did not rotate'
    assert observed.receipt is not None, 'new readiness has no strict receipt'
    validate_receipt(observed.receipt)
    assert 0 <= observed.ready_ns <= observed.complete_ns and 0 <= observed.retired_ns <= observed.complete_ns
    # Reject the old credential on the NEW listener, then explicitly reconnect.
    _,refused = call(target_for('gateway',service,after,token=before.token),'GET','/vms/list',cfg,deadline,status=401)
    assert refused == {'error':'unauthorized'}, 'old credential rejection contract differs'
    _,current = call(target_for('gateway',service,after),'GET','/vms/list',cfg,deadline)
    assert current == inventory, 'stopped inventory changed across restart'
    assert preserved_files(service,cfg) == registry, 'persistent registry changed across restart'
    completed = time.monotonic_ns()
    assert time.monotonic() <= deadline, 'restart effect verification exceeded deadline'
    prefix = f'{label}.{OPERATION} [managed-original].'
    samples = Observations()
    samples.append(receipt)
    metrics = samples.metrics(prefix,1)
    offset = observed_at-started
    clocks = (offset+observed.retired_ns,offset+observed.ready_ns,
        offset+observed.complete_ns,completed-started)
    for name,value in zip(CLOCKS,clocks,strict=True):
        metrics[prefix+name] = {'unit':'milliseconds','samples':[value/1_000_000]}
    for name,value in zip(PROBES,(observed.receipt.headers_ns,observed.receipt.first_body_byte_ns,observed.receipt.complete_ns),strict=True):
        assert value is not None
        metrics[prefix+name] = {'unit':'milliseconds','samples':[value/1_000_000]}
    metrics[prefix+'readiness_observations'] = {'unit':'count','samples':[len(observed.outcomes)]}
    return metrics


def require_restart_metrics(metrics,cfg):
    for label in ('service','gateway'):
        prefix = f'{label}.{OPERATION} [managed-original].'
        for name in ('latency_ms','headers_ms','first_body_byte_ms',*CLOCKS,*PROBES):
            metric = metrics.get(prefix+name,{})
            if metric.get('unit') != 'milliseconds' or len(metric.get('samples',[])) != cfg['cycles']:
                raise ValueError('missing or partial native restart clock: '+prefix+name)
        observations = metrics.get(prefix+'readiness_observations',{})
        if observations.get('unit') != 'count' or len(observations.get('samples',[])) != cfg['cycles']:
            raise ValueError('native restart readiness observations missing')
        for index in range(cfg['cycles']):
            wall = metrics.get(f'{label}.restart_controls.{index}.wall_s',{})
            if wall.get('unit') != 'seconds' or len(wall.get('samples',[])) != 1:
                raise ValueError('native restart owned resource phase missing')


@contextmanager
def restart_owner(cfg):
    service = ServiceInstance(isolated_corp_config=True)
    try:
        seed_stopped_workspace(service.tmp_dir,ASSETS_DIR,file_count=cfg['workspace_files'])
        with managed_service(service) as owner:
            yield owner,service
    finally:
        service.stop()


def collect_restart():
    cfg,metrics = restart_config(),{}
    for label in ('service','gateway'):
        for index in range(cfg['cycles']):
            with restart_owner(cfg) as (owner,service):
                initial = owner.ready_observed(None,policy=policy(cfg,time.monotonic()+cfg['timeout_seconds']))
                before = initial.identity
                _,inventory = call(target_for('gateway',service,before),'GET','/vms/list',cfg,
                    time.monotonic()+cfg['timeout_seconds'])
                rows = inventory.get('sandboxes',[])
                assert len(rows) == 1 and rows[0]['id'] == VM_ID and rows[0]['status'] == 'Stopped', 'original stopped fixture not ready'
                samples = measure(metrics,f'{label}.restart_controls.{index}',None,
                    partial(sample_restart,owner,service,before,inventory,cfg,label),
                    roots=owner.owned_roots,max_roots=cfg['max_roots'])
                for key,value in samples.items():
                    metrics.setdefault(key,{'unit':value['unit'],'samples':[]})['samples'].extend(value['samples'])
    require_restart_metrics(metrics,cfg)
    return metrics
