"""Original guest browser requests and byte uploads through existing owners."""
import json
import shlex
import time
import tomllib
from functools import partial
from pathlib import Path
from uuid import UUID

from psutil import Process

from .route_lifecycle import Cycle as LifecycleCycle
from .route_lifecycle import validate_exec
from .route_measurements import Observations, validate_receipt
from .route_requests import json_request
from .stream_measurements import measure, sample_count

RULE = 'route_workload_import'
PATTERN = b'Capsem bounded upload proof.\n'


def controls_config():
    from capsem_builder.gate.benchmarkcontrolschema import WorkloadControlConfig
    cfg = tomllib.loads((Path(__file__).resolve().parents[2]/'config/gate.toml').read_text())
    return WorkloadControlConfig.model_validate(cfg['benchmark']['routes']['workload_controls']).model_dump()


def upload_bytes(size):
    return (PATTERN*(size//len(PATTERN)+1))[:size]


class Cycle(LifecycleCycle):
    def call(self,method,path,body=None,*,deadline=None,expected_status=200):
        if expected_status == 200:
            return super().call(method,path,body,deadline=deadline)
        remaining = self.cfg['timeout_seconds'] if deadline is None else deadline-time.monotonic()
        if remaining <= 0:
            raise TimeoutError('control deadline exceeded')
        return json_request(self.transport,self.target,method,path,body,timeout=remaining,
            maximum=self.cfg['max_response_bytes'],expected_status=expected_status)

    def observe_control(self,operation,case,request,verify,*,deadline):
        started = time.monotonic()
        receipt,value = request()
        validate_receipt(receipt)
        result = verify(value)
        assert time.monotonic() <= deadline, 'control verification exceeded deadline'
        observations = Observations()
        observations.append(receipt)
        prefix = f'{self.target.label}.{operation} [{case}].'
        for key,metric in observations.metrics(prefix,1).items():
            self.metrics.setdefault(key,{'unit':metric['unit'],'samples':[]})['samples'].extend(metric['samples'])
        self.metrics.setdefault(prefix+'verified_completion_ms',{'unit':'milliseconds','samples':[]})['samples'].append((time.monotonic()-started)*1000)
        return result

    def browser(self,vm_id,*,deadline):
        while True:
            info = self.info(vm_id,deadline=deadline).get('browser_request')
            rows = [row for row in self.listing(deadline=deadline) if row['id'] == vm_id]
            assert len(rows) == 1, 'original browser owner disappeared'
            if rows[0].get('browser_request') == info:
                return info
            # Independent reads may straddle a genuine guest publication.
            if time.monotonic() >= deadline:
                raise TimeoutError('browser metadata diverges across public reads')
            time.sleep(self.cfg['poll_seconds'])

    def pending(self,vm_id,url,*,deadline):
        while True:
            value = self.browser(vm_id,deadline=deadline)
            if value is not None:
                assert value['url'] == url and type(value['id']) is int and value['id'] > 0
                assert str(UUID(value['owner_generation'])) == value['owner_generation']
                return value
            if time.monotonic() >= deadline:
                raise TimeoutError('genuine browser request not published')
            time.sleep(self.cfg['poll_seconds'])

    def ack(self,vm_id,value,*,deadline,status=200):
        body = {'owner_generation':value['owner_generation'],'request_id':value['id']}
        receipt,result = self.call('POST',f'/vms/{vm_id}/browser/acknowledge',body,deadline=deadline,expected_status=status)
        validate_receipt(receipt)
        assert receipt.status == status
        if status == 200:
            assert result == {'vm_id':vm_id,**body}, 'acknowledgement identity differs'
        return receipt,result

    def upload(self,vm_id,name,data,*,deadline):
        assert vm_id in self.owned and name == f'route-upload-{len(data)}.txt'
        remaining = deadline-time.monotonic()
        if remaining <= 0:
            raise TimeoutError('upload deadline exceeded')
        receipt = self.transport.measure_request('POST',f'/vms/{vm_id}/files/content?path={name}',
            headers={**self.target.headers,'Content-Type':'application/octet-stream','Accept':'application/json'},
            body=data,timeout=remaining,max_response_bytes=self.cfg['max_response_bytes'])
        validate_receipt(receipt)
        if receipt.status != 200:
            raise ValueError('original upload refused')
        return receipt,json.loads(receipt.body)


def sample_browser(cycle,vm_id):
    deadline = time.monotonic()+cycle.cfg['timeout_seconds']
    def opened(url):
        _,result = cycle.execute(vm_id,'xdg-open '+shlex.quote(url)+" && printf 'opened\\n'",deadline=deadline)
        validate_exec(result,'opened')
        return cycle.pending(vm_id,url,deadline=deadline)
    first = opened(cycle.cfg['browser_url'])
    def cleared(value):
        assert cycle.browser(vm_id,deadline=deadline) is None, 'original browser request remains pending'
    cycle.observe_control('POST /vms/{id}/browser/acknowledge','original-request-cleared',
        lambda:cycle.ack(vm_id,first,deadline=deadline),cleared,deadline=deadline)
    second = opened(cycle.cfg['browser_url']+'-newer')
    assert second['owner_generation'] == first['owner_generation'] and second['id'] == first['id']+1
    cycle.ack(vm_id,first,deadline=deadline)  # explicit untimed replay control
    assert cycle.browser(vm_id,deadline=deadline) == second, 'stale ack erased newer request'
    # Flip only the final bit: retain the real epoch's version/variant so the
    # public validator reaches owner conflict409, not malformed-body400.
    wrong = {**second,'owner_generation':str(UUID(int=UUID(second['owner_generation']).int ^ 1))}
    assert wrong['owner_generation'] != second['owner_generation']
    cycle.ack(vm_id,wrong,deadline=deadline,status=409)
    assert cycle.browser(vm_id,deadline=deadline) == second, 'wrong generation erased request'
    cycle.ack(vm_id,second,deadline=deadline)
    assert cycle.browser(vm_id,deadline=deadline) is None


def sample_upload(cycle,vm_id,size):
    deadline = time.monotonic()+cycle.cfg['timeout_seconds']
    name,body = f'route-upload-{size}.txt',upload_bytes(size)
    def verified(value):
        assert value == {'success':True,'size':size,'vm_path':'/root/'+name,'container_path':'/workspace/'+name}, 'upload acknowledgement differs'
        source = f"from pathlib import Path; data=Path('/workspace/{name}').read_bytes(); expected=({PATTERN!r}*({size}//{len(PATTERN)}+1))[:{size}]; assert data==expected; print('upload-verified')"
        _,result = cycle.execute(vm_id,'python3 -c '+shlex.quote(source),deadline=deadline)
        validate_exec(result,'upload-verified')
        listing = cycle.read(f'/vms/{vm_id}/files/list?depth=1',deadline=deadline)
        rows = [row for row in listing['entries'] if row['path'] == name]
        assert len(rows) == 1
        row = rows[0]
        assert row['name'] == name and row['type'] == 'file' and row['size'] == size
        assert row['mime'] == 'text/plain' and row['is_text'] is True
    cycle.observe_control('POST /vms/{id}/files/content',f'bytes-{size}',
        lambda:cycle.upload(vm_id,name,body,deadline=deadline),verified,deadline=deadline)


def sample_controls(cycle,reference,certificate):
    from .route_binary_fixture import verify_import_evidence
    cfg = cycle.cfg
    deadline = time.monotonic()+cfg['timeout_seconds']
    assert cycle.listing(deadline=deadline) == []
    _,value = cycle.call('POST','/vms/create',{'name':'route-workload-controls','persistent':True,
        'cpus':cfg['cpus'],'ram_mb':cfg['ram_mb'],'container':{'image':reference,
        'registry':{'ca_pem':certificate.read_text()}}},deadline=deadline)
    vm_id = cycle.identify(value)
    cycle.ready(vm_id,reference,deadline=deadline)
    sample_browser(cycle,vm_id)
    for size in cfg['file_sizes']:
        sample_upload(cycle,vm_id,size)
    # Existing normal stop is the producer drain barrier, never a timed upload
    # or replacement for physical-ledger proof in FilesProcessLedger.
    _,stopped = cycle.call('POST',f'/vms/{vm_id}/stop',{},deadline=time.monotonic()+cfg['timeout_seconds'])
    assert stopped['success'] is True and stopped['persistent'] is True
    verification_deadline = time.monotonic()+cfg['timeout_seconds']
    verify_import_evidence(lambda path:cycle.read(path,deadline=verification_deadline),vm_id,
        {f'route-upload-{size}.txt':size for size in cfg['file_sizes']},RULE,cfg['timeline_limit'])


def require_control_metrics(metrics,cfg,quick,labels=('service','gateway')):
    count = sample_count(cfg,'cycles',quick)
    for label in labels:
        for operation,entry in cfg['endpoints'].items():
            for case in entry['cases']:
                for suffix in ('latency_ms','headers_ms','first_body_byte_ms','verified_completion_ms'):
                    key = f'{label}.{operation} [{case}].{suffix}'
                    if key not in metrics or len(metrics[key]['samples']) != count:
                        raise ValueError('missing or partial workload control samples')
        for index in range(count):
            key = f'{label}.workload_controls.{index}.wall_s'
            if key not in metrics or len(metrics[key]['samples']) != 1:
                raise ValueError('missing or partial workload control resources')


def collect_workload_controls(quick):
    from helpers.settings_policy import write_settings_rule

    from .route_lifecycle_fixture import lifecycle_target
    cfg,metrics = controls_config(),{}
    for label in ('service','gateway'):
        for index in range(sample_count(cfg,'cycles',quick)):
            with lifecycle_target(label) as (target,service,reference,certificate):
                names = [f'route-upload-{size}.txt' for size in cfg['file_sizes']]
                write_settings_rule(service.home_dir,RULE,action='allow',detection_level='informational',
                    match='file.import.name in '+json.dumps(names),reason='Isolated synthetic benchmark upload corpus.')
                transport = target.transport_factory()
                cycle = Cycle(transport,target,service,cfg,metrics)
                try:
                    assert service.proc is not None
                    measure(metrics,f'{label}.workload_controls.{index}',Process(service.proc.pid),
                        partial(sample_controls,cycle,reference,certificate))
                    cycle.cleanup()
                    assert cycle.listing() == []
                finally:
                    transport.close()
    require_control_metrics(metrics,cfg,quick)
    return metrics
