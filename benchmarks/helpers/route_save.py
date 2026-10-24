"""Original live persistence claim; cold retention verification is untimed."""
import shlex
import time
import tomllib
from functools import partial
from pathlib import Path
from uuid import UUID

from helpers.service import exec_output_text
from psutil import Process

from .route_credentials import require_credential_metrics
from .route_lifecycle import validate_exec
from .route_preview import publication_generation
from .route_run_purge import read_file
from .route_workload_controls import Cycle
from .stream_measurements import measure, sample_count

FILE = 'capsem-route-save-proof'
TRANSIENT = '/run/capsem-route-save-runtime'


def save_config():
    from capsem_builder.gate.benchmarksaveschema import SaveRouteConfig
    raw = tomllib.loads((Path(__file__).resolve().parents[2]/'config/gate.toml').read_text())
    return SaveRouteConfig.model_validate(raw['benchmark']['routes']['save_controls']).model_dump()


def path_identity(path):
    stat = path.stat()
    assert path.is_dir()
    return stat.st_dev,stat.st_ino


def vm_text(cycle,vm_id,command,deadline):
    _,value = cycle.call('POST',f'/vms/{vm_id}/exec',{'command':command,
        'target':'vm','timeout_secs':cycle.cfg['exec_timeout_seconds']},deadline=deadline)
    assert value['exit_code'] == 0 and value['truncated'] is False
    assert exec_output_text(value,'stderr') == ''
    return exec_output_text(value)


def boot_id(text):
    value = text.strip()
    assert str(UUID(value)) == value and UUID(value).int != 0
    return value


def generation(cycle,vm_id,deadline):
    value = cycle.read(f'/vms/{vm_id}/exposures',deadline=deadline)
    assert isinstance(value['exposures'],list)
    return publication_generation(value['owner_generation'])


def verify_bytes(cycle,vm_id,deadline):
    _,value = cycle.execute(vm_id,f'cat /workspace/{FILE}',deadline=deadline)
    validate_exec(value,cycle.cfg['marker'])
    read_file(cycle,vm_id,FILE,(cycle.cfg['marker']+'\n').encode(),deadline)


def live(cycle,vm_id,process,expected_name,deadline):
    info = cycle.info(vm_id,deadline=deadline)
    assert info['id'] == vm_id and info['status'] == 'Running' and info['persistent'] is True
    assert info['name'] == expected_name and info['pid'] == process.pid and process.is_running()
    current = Process(info['pid'])
    assert (current.pid,current.create_time()) == (process.pid,process.create_time()), 'original live owner replaced'
    rows = cycle.listing(deadline=deadline)
    assert len(rows) == 1 and rows[0]['id'] == vm_id and rows[0]['name'] == expected_name and rows[0]['persistent'] is True


def sample_save(cycle,reference,certificate):
    cfg = cycle.cfg
    deadline = time.monotonic()+cfg['timeout_seconds']
    assert cycle.listing(deadline=deadline) == [] and not cycle.owned
    # A name makes create persistent. Explicitly omit it and request ephemeral.
    _,value = cycle.call('POST','/vms/create',{'persistent':False,'cpus':cfg['cpus'],'ram_mb':cfg['ram_mb'],
        'container':{'image':reference,'registry':{'ca_pem':certificate.read_text()}}},deadline=deadline)
    vm_id = cycle.identify(value)
    info = cycle.ready(vm_id,reference,deadline=deadline,persistent=False)
    original_generation = generation(cycle,vm_id,deadline)
    original = Process(info['pid'])
    original_path = cycle.session(vm_id,deadline=deadline)
    original_identity = path_identity(original_path)
    assert original_path.parent == cycle.service.tmp_dir/'sessions', 'source is not actual ephemeral storage'
    _,written = cycle.execute(vm_id,f"printf '%s\\n' {shlex.quote(cfg['marker'])} > /workspace/{FILE}; cat /workspace/{FILE}",deadline=deadline)
    validate_exec(written,cfg['marker'])
    verify_bytes(cycle,vm_id,deadline)
    before_boot = boot_id(vm_text(cycle,vm_id,f"printf live > {TRANSIENT}; cat /proc/sys/kernel/random/boot_id",deadline))
    def claimed(value):
        assert value == {'success':True,'name':cfg['name']}
        live(cycle,vm_id,original,cfg['name'],deadline)
        assert generation(cycle,vm_id,deadline) == original_generation, 'save replaced the actual spawn generation'
        assert cycle.session(vm_id,deadline=deadline) == original_path, 'save moved path under live process'
        assert path_identity(original_path) == original_identity
        assert not (cycle.service.tmp_dir/'persistent'/vm_id).exists(), 'save published a second live session'
        verify_bytes(cycle,vm_id,deadline)
    cycle.observe_control('POST /vms/{id}/save','original-live-claim',
        lambda:cycle.call('POST',f'/vms/{vm_id}/save',{'name':cfg['name']},deadline=deadline),claimed,deadline=deadline)
    # Mandatory cold retention checks are outside save response/claim clocks;
    # the existing process-tree wall/resource phase includes the whole cycle.
    _,stopped = cycle.call('POST',f'/vms/{vm_id}/stop',{},deadline=deadline)
    assert stopped == {'success':True,'persistent':True} and not original.is_running()
    retained = cycle.info(vm_id,deadline=deadline)
    assert retained['status'] == 'Stopped' and retained['persistent'] is True and retained['can_resume'] is True
    assert retained['id'] == vm_id and retained['name'] == cfg['name']
    moved = cycle.session(vm_id,deadline=deadline)
    assert moved.parent == cycle.service.tmp_dir/'persistent' and moved.name == vm_id
    assert moved != original_path and not original_path.exists() and path_identity(moved) == original_identity
    _,started = cycle.call('POST',f'/vms/{vm_id}/start',{},deadline=deadline)
    assert started['id'] == vm_id and started['persistent'] is True
    running = cycle.ready(vm_id,reference,deadline=deadline)
    replacement = Process(running['pid'])
    assert (replacement.pid,replacement.create_time()) != (original.pid,original.create_time()) and not original.is_running()
    live(cycle,vm_id,replacement,cfg['name'],deadline)
    assert generation(cycle,vm_id,deadline) != original_generation, 'cold start reused the original retired spawn generation'
    assert cycle.session(vm_id,deadline=deadline) == moved and path_identity(moved) == original_identity
    after_boot = boot_id(vm_text(cycle,vm_id,'cat /proc/sys/kernel/random/boot_id',deadline))
    assert after_boot != before_boot, 'warm runtime substituted for normal cold start'
    assert vm_text(cycle,vm_id,f"test ! -e {TRANSIENT} && printf 'cold-runtime\\n'",deadline) == 'cold-runtime\n'
    verify_bytes(cycle,vm_id,deadline)
    _,stopped = cycle.call('POST',f'/vms/{vm_id}/stop',{},deadline=deadline)
    assert stopped == {'success':True,'persistent':True} and not replacement.is_running()
    _,deleted = cycle.call('DELETE',f'/vms/{vm_id}/delete',deadline=deadline)
    assert deleted == {'success':True}
    cycle.absent(vm_id,moved,replacement,deadline=deadline)


def require_save_metrics(metrics,cfg,quick,labels=('service','gateway')):
    require_credential_metrics(metrics,cfg,quick,labels,resource_domain='save_controls')


def prepare_policy(service):
    from helpers.settings_policy import write_settings_rule
    write_settings_rule(service.home_dir,'route_save_export',action='allow',detection_level='informational',
        match='file.export.name == "'+FILE+'"',reason='Exact isolated synthetic persistence proof file only.')


def collect_save_controls(quick):
    from .route_lifecycle_fixture import lifecycle_target
    cfg,metrics = save_config(),{}
    for label in ('service','gateway'):
        for index in range(sample_count(cfg,'cycles',quick)):
            with lifecycle_target(label,prepare=prepare_policy) as (target,service,reference,certificate):
                transport = target.transport_factory()
                cycle = Cycle(transport,target,service,cfg,metrics)
                try:
                    assert service.proc is not None
                    measure(metrics,f'{label}.save_controls.{index}',Process(service.proc.pid),partial(sample_save,cycle,reference,certificate))
                    cycle.cleanup()
                    assert cycle.listing() == []
                finally:
                    transport.close()
    require_save_metrics(metrics,cfg,quick)
    return metrics
