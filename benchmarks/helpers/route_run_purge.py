"""VM-only one-shot execution and owned stopped disposal; source is not timing."""
import shlex
import time
import tomllib
from functools import partial
from pathlib import Path

from psutil import Process

from .route_credentials import require_credential_metrics
from .route_lifecycle import validate_exec
from .route_measurements import validate_receipt
from .route_workload_controls import Cycle
from .stream_measurements import measure, sample_count

FILE = 'capsem-route-purge-proof'
RUN_FILE = 'capsem-route-run-proof'
RELEASE = 'capsem-route-run-release'


def run_purge_config():
    from capsem_builder.gate.benchmarkrunpurgeschema import RunPurgeConfig
    raw = tomllib.loads((Path(__file__).resolve().parents[2]/'config/gate.toml').read_text())
    return RunPurgeConfig.model_validate(raw['benchmark']['routes']['run_purge']).model_dump()


def expected_marker(kind,cfg=None):
    cfg = run_purge_config() if cfg is None else cfg
    return (cfg['marker']+':'+kind+'\n').encode()


def read_file(cycle,vm_id,name,expected,deadline):
    receipt = cycle.transport.measure_request('GET',f'/vms/{vm_id}/files/content?path={name}',
        headers=cycle.target.headers,timeout=deadline-time.monotonic(),max_response_bytes=cycle.cfg['max_response_bytes'])
    validate_receipt(receipt)
    assert receipt.status == 200 and receipt.body == expected, 'independent original file bytes differ'


def sample_purge(cycle,reference,certificate):
    cfg = cycle.cfg
    deadline = time.monotonic()+cfg['timeout_seconds']
    assert cycle.listing(deadline=deadline) == [], 'purge requires isolated empty ownership'
    originals = []
    for index in range(cfg['retained_sessions']):
        _,value = cycle.call('POST','/vms/create',{'name':f'route-purge-{index}','persistent':True,
            'cpus':cfg['cpus'],'ram_mb':cfg['ram_mb'],'container':{'image':reference,
            'registry':{'ca_pem':certificate.read_text()}}},deadline=deadline)
        vm_id = cycle.identify(value)
        info = cycle.ready(vm_id,reference,deadline=deadline)
        process = Process(info['pid'])
        path = cycle.session(vm_id,deadline=deadline)
        marker = expected_marker('purge',cfg).decode().rstrip('\n')
        _,output = cycle.execute(vm_id,f"printf '%s\\n' {shlex.quote(marker)} > /workspace/{FILE}; cat /workspace/{FILE}",deadline=deadline)
        validate_exec(output,marker)
        read_file(cycle,vm_id,FILE,expected_marker('purge',cfg),deadline)
        _,stopped = cycle.call('POST',f'/vms/{vm_id}/stop',{},deadline=deadline)
        assert stopped == {'success':True,'persistent':True}
        assert not process.is_running() and path.is_dir()
        originals.append((vm_id,path,process))
    def retained():
        assert {row['id'] for row in cycle.listing(deadline=deadline)} == cycle.owned
        for vm_id,path,process in originals:
            info = cycle.info(vm_id,deadline=deadline)
            assert info['status'] == 'Stopped' and info['persistent'] is True and info['can_resume'] is True
            assert not process.is_running() and path.is_dir()
            entries = cycle.read(f'/vms/{vm_id}/files/list?depth=1',deadline=deadline)['entries']
            selected = [row for row in entries if row['path'] == FILE]
            assert len(selected) == 1 and selected[0]['type'] == 'file' and selected[0]['size'] == len(expected_marker('purge',cfg))
    retained()
    # Untimed negative control: a no-op default response is not measured purge
    # coverage. Independently prove healthy stopped sessions remain intact.
    _,default = cycle.call('POST','/purge',{'all':False},deadline=deadline)
    assert default == {'purged':0,'persistent_purged':0,'ephemeral_purged':0}
    retained()
    def removed(value):
        assert value == {'purged':len(originals),'persistent_purged':len(originals),'ephemeral_purged':0}
        for vm_id,path,process in originals:
            cycle.absent(vm_id,path,process,deadline=deadline)
        assert cycle.listing(deadline=deadline) == [] and not cycle.owned
    cycle.observe_control('POST /purge','owned-stopped-disposal',
        lambda:cycle.call('POST','/purge',{'all':True},deadline=deadline),removed,deadline=deadline)


def require_run_purge_metrics(metrics,cfg,quick,labels=('service','gateway')):
    require_credential_metrics(metrics,cfg,quick,labels,resource_domain='run_purge')


def collect_run_purge(quick):
    from .route_lifecycle_fixture import lifecycle_target
    from .route_run_probe import sample_run
    cfg,metrics = run_purge_config(),{}
    for label in ('service','gateway'):
        for index in range(sample_count(cfg,'cycles',quick)):
            with lifecycle_target(label,prepare=prepare_policy) as (target,service,reference,certificate):
                transport = target.transport_factory()
                cycle = Cycle(transport,target,service,cfg,metrics)
                try:
                    assert service.proc is not None
                    measure(metrics,f'{label}.run_purge.{index}',Process(service.proc.pid),
                        partial(sample_cycle,cycle,reference,certificate,sample_run))
                    cycle.cleanup()
                    assert cycle.listing() == []
                finally:
                    transport.close()
    require_run_purge_metrics(metrics,cfg,quick)
    return metrics


def sample_cycle(cycle,reference,certificate,run):
    run(cycle)
    sample_purge(cycle,reference,certificate)


def prepare_policy(service):
    from helpers.settings_policy import file_name_condition, write_settings_rule
    for action,names in (('export',[FILE,RUN_FILE]),('import',[RELEASE])):
        write_settings_rule(service.home_dir,'route_run_purge_'+action,action='allow',
            detection_level='informational',match=file_name_condition(action,names),
            reason='Exact isolated synthetic run/purge fixture files only.')
