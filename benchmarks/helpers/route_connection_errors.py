"""Explicit typed refusals on matched routes, not successful enrollment timing."""
import json
import tomllib
from pathlib import Path

import tomli_w
from helpers.stopped_workspace import VM_ID, seed_stopped_workspace
from psutil import Process

from .route_measurements import Observations, validate_receipt
from .route_requests import json_request
from .stream_measurements import measure, sample_count


def error_config():
    from capsem_builder.gate.benchmarkerrorschema import ConnectionErrorConfig
    raw = tomllib.loads((Path(__file__).resolve().parents[2]/'config/gate.toml').read_text())
    return ConnectionErrorConfig.model_validate(raw['benchmark']['routes']['connection_errors']).model_dump()


def requests(cfg):
    result = {}
    for operation in cfg['endpoints']:
        method,path = operation.split(' ',1)
        path = path.format(id=VM_ID,transaction_id=cfg['transaction_id'],connection_id=cfg['connection_id'],grant_id=cfg['grant_id'])
        body = None
        if operation == 'POST /connections/google/authorize':
            body = {'request_id':cfg['request_id'],'scope_profile_id':'agy-native','reconnect_connection_id':None}
        elif operation == 'POST /vms/{id}/credential-grants':
            body = {'request_id':cfg['request_id'],'expected_owner_generation':VM_ID,
                    'connection_id':cfg['connection_id'],'capability_ids':['agy'],'expires_at':None}
        result[operation] = method,path,body
    return result


def validate_error(receipt,entry):
    validate_receipt(receipt)
    assert receipt.status == entry['status'], 'wrong error-path HTTP classification'
    assert receipt.headers.get('content-type','').split(';')[0] == 'application/json', 'matched route must return typed JSON refusal'
    value = json.loads(receipt.body)
    assert value == {'error':'credential request refused','code':entry['code']}, 'wrong typed refusal or private input echoed'


def sample_errors(transport,target,cfg,*,quick):
    metrics = {}
    for operation,(method,path,body) in requests(cfg).items():
        entry = cfg['endpoints'][operation]
        observations = Observations()
        for _ in range(sample_count(cfg,'samples',quick)):
            receipt,_ = json_request(transport,target,method,path,body,
                timeout=cfg['timeout_seconds'],maximum=cfg['max_response_bytes'],expected_status=entry['status'])
            validate_error(receipt,entry)
            observations.append(receipt)
        prefix = f"{target.label}.error_path.{operation} [{entry['classification']};http{entry['status']};{entry['code']}]."
        metrics.update(observations.metrics(prefix,sample_count(cfg,'samples',quick)))
    return metrics


def prepare(service):
    # Existing stopped-workspace corpus is registry input, not a running VM,
    # connection, grant, credential or invented enrollment authority.
    seed_stopped_workspace(service.tmp_dir,service.assets_dir,file_count=1)
    (service.home_dir/'corp.toml').write_text(tomli_w.dumps({'refresh_policy':'24h','plugins':{
        'credential_broker':{'mode':'rewrite','detection_level':'informational'}}}))


def check_empty_authority(transport,target,cfg):
    def read(path):
        return json_request(transport,target,'GET',path,None,timeout=cfg['timeout_seconds'],maximum=cfg['max_response_bytes'])[1]
    assert read('/version')['version'], 'authenticated public gateway preflight failed'
    catalog = read('/connections')
    assert catalog['connections'] == [] and catalog['next_cursor'] is None
    assert len(catalog['scope_profiles']) == 1
    profile = catalog['scope_profiles'][0]
    assert profile['id'] == 'agy-native' and profile['enabled'] is False
    assert profile['capabilities'] and all(entry['supported'] is False for entry in profile['capabilities'])
    rows = read('/vms/list')['sandboxes']
    assert len(rows) == 1 and rows[0]['id'] == VM_ID and rows[0]['status'] == 'Stopped'
    grants = read(f'/vms/{VM_ID}/credential-grants')
    assert grants == {'session_id':VM_ID,'owner_generation':VM_ID,'grants':[],'denied_capability_ids':[]}


def require_error_metrics(metrics,cfg,quick,labels=('service','gateway')):
    expected = sample_count(cfg,'samples',quick)
    for label in labels:
        for operation,entry in cfg['endpoints'].items():
            prefix = f"{label}.error_path.{operation} [{entry['classification']};http{entry['status']};{entry['code']}]."
            for suffix in ('latency_ms','headers_ms','first_body_byte_ms'):
                key = prefix+suffix
                if key not in metrics or len(metrics[key]['samples']) != expected:
                    raise ValueError('missing or partial explicitly classified error-path samples')
        key = label+'.connection_error_paths.wall_s'
        if key not in metrics or len(metrics[key]['samples']) != 1:
            raise ValueError('missing or partial error-path resource receipt')


def collect_connection_errors(quick):
    from .route_network_fixture import network_target
    cfg,metrics = error_config(),{}
    for label in ('service','gateway'):
        with network_target(label,prepare=prepare) as (target,service):
            transport = target.transport_factory()
            try:
                check_empty_authority(transport,target,cfg)
                assert service.proc is not None
                def observed(transport=transport,target=target):
                    metrics.update(sample_errors(transport,target,cfg,quick=quick))
                measure(metrics,label+'.connection_error_paths',Process(service.proc.pid),observed)
                check_empty_authority(transport,target,cfg)  # refusal must not create authority
            finally:
                transport.close()
    require_error_metrics(metrics,cfg,quick)
    return metrics
