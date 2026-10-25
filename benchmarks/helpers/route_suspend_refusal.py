"""Measure disabled warm operations, never claim a restored guest (#378)."""
import json
import time
from pathlib import Path

from helpers.stopped_workspace import VM_ID, VM_NAME, seed_stopped_workspace
from psutil import Process

from .route_save import boot_id, generation, path_identity, vm_text

SAVED = b"synthetic unavailable checkpoint; not native VM state\n"


def prepare_saved_session(service):
    """Owned synthetic saved bytes test refusal, not native restore validity."""
    seed_stopped_workspace(service.tmp_dir, service.assets_dir, file_count=1)
    registry = service.tmp_dir / 'persistent_registry.json'
    value = json.loads(registry.read_text())
    entry = value['vms'][VM_NAME]
    entry.update(suspended=True, checkpoint_path='owner/checkpoint.vzsave')
    owner = Path(entry['session_dir']) / 'owner'
    owner.mkdir(mode=0o700)
    for name, data in [('checkpoint.vzsave', SAVED), ('checkpoint.vzsave.complete', b'ok\n')]:
        path = owner / name
        path.write_bytes(data)
        path.chmod(0o600)
    registry.write_text(json.dumps(value))


def refusal(value):
    assert set(value) == {'error'} and 'warm suspend is disabled' in value['error'], 'wrong unsupported-operation refusal'


def sample_disabled_suspend(cycle, reference, certificate):
    deadline = time.monotonic() + cycle.cfg['timeout_seconds']
    assert [row['id'] for row in cycle.listing(deadline=deadline)] == [VM_ID]
    cycle.owned.add(VM_ID)
    saved = cycle.service.tmp_dir / 'persistent' / VM_ID
    saved_identity = path_identity(saved)
    saved_info = cycle.info(VM_ID, deadline=deadline)
    assert saved_info['persistent'] and not saved_info['can_resume']
    assert 'resume' not in saved_info['available_actions']
    _, value = cycle.call('POST', '/vms/create', {'name': cycle.cfg['name'], 'persistent': True,
        'cpus': cycle.cfg['cpus'], 'ram_mb': cycle.cfg['ram_mb'],
        'container': {'image': reference, 'registry': {'ca_pem': certificate.read_text()}}}, deadline=deadline)
    vm_id = cycle.identify(value)
    info = cycle.ready(vm_id, reference, deadline=deadline)
    original = Process(info['pid'])
    path = cycle.session(vm_id, deadline=deadline)
    identity = path_identity(path)
    owner_generation = generation(cycle, vm_id, deadline)
    boot = boot_id(vm_text(cycle, vm_id, 'cat /proc/sys/kernel/random/boot_id', deadline))

    def unchanged_owner(value):
        refusal(value)
        current = cycle.info(vm_id, deadline=deadline)
        assert current['status'] == 'Running' and current['pid'] == original.pid and original.is_running()
        assert 'pause' not in current['available_actions']
        assert generation(cycle, vm_id, deadline) == owner_generation
        assert path_identity(path) == identity
        assert boot_id(vm_text(cycle, vm_id, 'cat /proc/sys/kernel/random/boot_id', deadline)) == boot
        assert not (path / 'owner/checkpoint.vzsave.complete').exists()

    cycle.observe_control('POST /vms/{id}/pause', 'disabled-owner-unchanged',
        lambda: cycle.call('POST', f'/vms/{vm_id}/pause', {}, deadline=deadline, expected_status=501),
        unchanged_owner, deadline=deadline)

    def unchanged_checkpoint(value):
        refusal(value)
        current = cycle.info(VM_ID, deadline=deadline)
        for key in ('status', 'can_resume', 'available_actions'):
            assert current[key] == saved_info[key]
        assert path_identity(saved) == saved_identity
        assert (saved / 'owner/checkpoint.vzsave').read_bytes() == SAVED
        assert (saved / 'owner/checkpoint.vzsave.complete').read_bytes() == b'ok\n'
        assert original.is_running() and cycle.info(vm_id, deadline=deadline)['pid'] == original.pid

    cycle.observe_control('POST /vms/{id}/resume', 'disabled-checkpoint-preserved',
        lambda: cycle.call('POST', f'/vms/{VM_ID}/resume', {}, deadline=deadline, expected_status=501),
        unchanged_checkpoint, deadline=deadline)
    _, stopped = cycle.call('POST', f'/vms/{vm_id}/stop', {}, deadline=deadline)
    assert stopped == {'success': True, 'persistent': True} and not original.is_running()
    _, deleted = cycle.call('DELETE', f'/vms/{vm_id}/delete', deadline=deadline)
    assert deleted == {'success': True}
    cycle.absent(vm_id, path, original, deadline=deadline)
    _, deleted = cycle.call('DELETE', f'/vms/{VM_ID}/delete', deadline=deadline)
    assert deleted == {'success': True} and not saved.exists()
    cycle.owned.discard(VM_ID)
    assert cycle.listing(deadline=deadline) == []
