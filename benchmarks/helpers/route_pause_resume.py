"""Disabled warm-operation measurements and retained #378 regression probes."""
import time
import tomllib
from functools import partial
from pathlib import Path

from psutil import Process

from .route_credentials import require_credential_metrics
from .route_run_purge import read_file
from .route_save import boot_id, generation, path_identity, vm_text
from .route_suspend_refusal import prepare_saved_session, sample_disabled_suspend
from .route_warm_probe import FILE, ORIGINAL, RESUMED, RUNTIME, WarmProbe
from .route_workload_controls import Cycle
from .stream_measurements import measure, sample_count


def pause_resume_config():
    from capsem_builder.gate.benchmarkwarmschema import PauseResumeConfig
    raw = tomllib.loads((Path(__file__).resolve().parents[2]/'config/gate.toml').read_text())
    return PauseResumeConfig.model_validate(raw['benchmark']['routes']['pause_resume']).model_dump()


def checkpoint(path,cfg):
    owner = path/'owner'
    assert owner.is_dir() and not owner.is_symlink(), 'checkpoint owner directory invalid'
    for name,maximum in (('checkpoint.vzsave',cfg['max_checkpoint_bytes']),
        ('terminal/checkpoint.msgpack',cfg['max_terminal_checkpoint_bytes'])):
        artifact = owner/name
        assert not artifact.parent.is_symlink() and artifact.is_file() and not artifact.is_symlink()
        assert 0 < artifact.stat().st_size <= maximum, 'checkpoint absent or exceeds bound'
    marker = owner/'checkpoint.vzsave.complete'
    assert marker.is_file() and not marker.is_symlink()
    with marker.open('rb') as source:
        assert source.read(cfg['max_metadata_bytes']+1) == b'ok\n', 'native checkpoint not complete'


def original_state(cycle,vm_id,status,deadline):
    info = cycle.info(vm_id,deadline=deadline)
    rows = [row for row in cycle.listing(deadline=deadline) if row['id'] == vm_id]
    assert len(rows) == 1 and rows[0]['status'] == info['status'] == status
    assert info['id'] == vm_id and info['persistent'] is True and info['name'] == cycle.cfg['name']
    assert cycle.read(f'/vms/{vm_id}/status',deadline=deadline)['status'] == status
    return info


def sample_original_warm(cycle,reference,certificate):
    """Retained regression only; the active collector measures disabled refusals."""
    cfg = cycle.cfg
    deadline = time.monotonic()+cfg['timeout_seconds']
    assert cycle.listing(deadline=deadline) == []
    _,value = cycle.call('POST','/vms/create',{'name':cfg['name'],'persistent':True,
        'cpus':cfg['cpus'],'ram_mb':cfg['ram_mb'],
        'container':{'image':reference,'registry':{'ca_pem':certificate.read_text()}}},deadline=deadline)
    vm_id = cycle.identify(value)
    info = cycle.ready(vm_id,reference,deadline=deadline)
    original = Process(info['pid'])
    path = cycle.session(vm_id,deadline=deadline)
    identity = path_identity(path)
    before_generation = generation(cycle,vm_id,deadline)
    probe = WarmProbe(cycle,vm_id)
    before_boot = boot_id(vm_text(cycle,vm_id,
        f'printf {probe.nonce} > {RUNTIME}; cat /proc/sys/kernel/random/boot_id',deadline))
    probe.start(deadline)
    read_file(cycle,vm_id,FILE,ORIGINAL,deadline)
    def paused(value):
        assert value == {'success':True}, 'pause acknowledgement differs'
        info = original_state(cycle,vm_id,'Suspended',deadline)
        assert info['can_resume'] is True and not original.is_running(), 'original owner not retired'
        assert cycle.session(vm_id,deadline=deadline) == path and path_identity(path) == identity
        checkpoint(path,cfg)
    cycle.observe_control('POST /vms/{id}/pause','original-checkpoint',
        lambda:cycle.call('POST',f'/vms/{vm_id}/pause',{},deadline=deadline),paused,deadline=deadline)
    def resumed(value):
        assert value['id'] == vm_id and value['persistent'] is True, 'resume identity differs'
        running = original_state(cycle,vm_id,'Running',deadline)
        replacement = Process(running['pid'])
        assert (replacement.pid,replacement.create_time()) != (original.pid,original.create_time())
        assert replacement.is_running() and not original.is_running(), 'host owner did not replace original'
        assert cycle.session(vm_id,deadline=deadline) == path and path_identity(path) == identity
        assert generation(cycle,vm_id,deadline) != before_generation
        assert not (path/'owner/checkpoint.vzsave.complete').exists(), 'resume retained committed checkpoint marker'
        # As in canonical terminal_suspend, prove retained descriptors BEFORE
        # the first fresh VM/workload exec could repair or mask runtime state.
        probe.resumed(deadline)
        cycle.ready(vm_id,reference,deadline=deadline)
        after_boot = boot_id(vm_text(cycle,vm_id,
            f'test "$(cat {RUNTIME})" = {probe.nonce}; cat /proc/sys/kernel/random/boot_id',deadline))
        assert after_boot == before_boot, 'cold boot substituted for warm resume'
        read_file(cycle,vm_id,FILE,RESUMED,deadline)
    cycle.observe_control('POST /vms/{id}/resume','original-warm-continuation',
        lambda:cycle.call('POST',f'/vms/{vm_id}/resume',{},deadline=deadline),resumed,deadline=deadline)
    current = Process(cycle.info(vm_id,deadline=deadline)['pid'])
    _,stopped = cycle.call('POST',f'/vms/{vm_id}/stop',{},deadline=deadline)
    assert stopped == {'success':True,'persistent':True} and not current.is_running()
    _,deleted = cycle.call('DELETE',f'/vms/{vm_id}/delete',deadline=deadline)
    assert deleted == {'success':True}
    cycle.absent(vm_id,path,current,deadline=deadline)


def require_pause_metrics(metrics,cfg,quick):
    require_credential_metrics(metrics,cfg,quick,resource_domain='pause_resume')


def prepare_policy(service):
    from helpers.settings_policy import write_settings_rule
    write_settings_rule(service.home_dir,'route_warm_export',action='allow',detection_level='informational',
        match='file.export.name == "'+FILE+'"',reason='Only the exact synthetic original warm continuation file.')


def collect_pause_resume(quick):
    from .route_lifecycle_fixture import lifecycle_target
    cfg,metrics = pause_resume_config(),{}
    for label in ('service','gateway'):
        for index in range(sample_count(cfg,'cycles',quick)):
            with lifecycle_target(label,prepare=prepare_saved_session) as (target,service,reference,certificate):
                transport = target.transport_factory()
                cycle = Cycle(transport,target,service,cfg,metrics)
                try:
                    assert service.proc is not None
                    measure(metrics,f'{label}.pause_resume.{index}',Process(service.proc.pid),
                        partial(sample_disabled_suspend,cycle,reference,certificate))
                    cycle.cleanup()
                    assert cycle.listing() == []
                finally:
                    transport.close()
    require_pause_metrics(metrics,cfg,quick)
    return metrics
