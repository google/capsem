"""One original VM-only run, fenced by independent public file observation."""
import json
import re
import shlex
import time
from concurrent.futures import ThreadPoolExecutor

from helpers.service import exec_output_text
from psutil import Process

from .route_measurements import validate_receipt
from .route_run_purge import RELEASE, RUN_FILE, expected_marker, read_file


def command(cfg):
    # RunRequest has no workload/container target. UID/map belong to the VM.
    source = f'''
import os,time
from pathlib import Path
root=Path('/root')
proof=root/{RUN_FILE!r}
release=root/{RELEASE!r}
mapping=','.join(Path('/proc/self/uid_map').read_text().split())
proof.write_text(str(os.getuid())+':'+str(os.getgid())+':'+mapping+'\\n'+os.environ['ROUTE_RUN_MARKER']+':run\\n')
deadline=time.monotonic()+{cfg['exec_timeout_seconds']!r}
while not release.exists():
    if time.monotonic() >= deadline: raise TimeoutError('original observer did not release run')
    time.sleep({cfg['poll_seconds']!r})
assert release.read_bytes()==b'release-original-run\\n'
print('run-complete')
'''
    compile(source,'route-run-fixture','exec')
    return 'python3 -c '+shlex.quote(source)


def wait_original(cycle,future,deadline):
    while True:
        rows = cycle.listing(deadline=deadline)
        assert len(rows) <= 1, 'run fixture acquired unrelated owners'
        if rows:
            row = rows[0]
            assert row['persistent'] is False and re.fullmatch(r'[A-Za-z0-9_-]{1,64}',row['id'])
            if row['status'] == 'Running':
                assert type(row['pid']) is int and row['pid'] > 0
                cycle.owned.add(row['id'])
                return row
        if future.done():
            future.result()  # propagate original refusal/timeout, never retry
            raise AssertionError('run completed before independent original effect was observed')
        if time.monotonic() >= deadline:
            raise TimeoutError('original run owner did not publish')
        time.sleep(cycle.cfg['poll_seconds'])


def observe_effect(cycle,future,deadline):
    row = wait_original(cycle,future,deadline)
    vm_id = row['id']
    process = Process(row['pid'])  # retains original PID creation identity
    assert process.is_running()
    path = cycle.session(vm_id,deadline=deadline)
    while True:
        listing = cycle.read(f'/vms/{vm_id}/files/list?depth=1',deadline=deadline)
        if any(entry['path'] == RUN_FILE for entry in listing['entries']):
            break
        if future.done() or time.monotonic() >= deadline:
            raise AssertionError('original run never produced its file effect')
        time.sleep(cycle.cfg['poll_seconds'])
    expected = b'0:0:0,0,4294967295\n'+expected_marker('run',cycle.cfg)
    read_file(cycle,vm_id,RUN_FILE,expected,deadline)
    assert process.is_running(), 'original owner exited before release'
    # Public original upload is the explicit observer fence, not a host write
    # into a guest directory or an additional command/execution authority.
    body = b'release-original-run\n'
    result = cycle.transport.measure_request('POST',f'/vms/{vm_id}/files/content?path={RELEASE}',
        headers={**cycle.target.headers,'Content-Type':'application/octet-stream'},body=body,
        timeout=deadline-time.monotonic(),max_response_bytes=cycle.cfg['max_response_bytes'])
    validate_receipt(result)
    assert result.status == 200
    uploaded = json.loads(result.body)
    # Current UploadResponse omits its None container_path; this VM-only
    # request must not claim a workload mapping or legacy null-shaped reply.
    assert uploaded == {'success':True,'size':len(body),'vm_path':'/root/'+RELEASE}
    return vm_id,path,process


def sample_run(cycle):
    cfg = cycle.cfg
    deadline = time.monotonic()+cfg['timeout_seconds']
    assert cycle.listing(deadline=deadline) == [] and not cycle.owned
    sessions = cycle.service.tmp_dir/'sessions'
    original_paths = set(sessions.iterdir()) if sessions.exists() else set()
    assert not original_paths, 'run requires isolated empty session storage'
    observed = None
    def request():
        nonlocal observed
        with ThreadPoolExecutor(max_workers=1) as executor:
            future = executor.submit(cycle.call,'POST','/run',{'command':command(cfg),
                'timeout_secs':cfg['exec_timeout_seconds'],'ram_mb':cfg['ram_mb'],'cpus':cfg['cpus'],
                'env':{'ROUTE_RUN_MARKER':cfg['marker']}},deadline=deadline)
            observed = observe_effect(cycle,future,deadline)
            return future.result(timeout=max(0,deadline-time.monotonic()))
    def completed(value):
        assert value['exit_code'] == 0 and value['truncated'] is False
        assert exec_output_text(value) == 'run-complete\n' and exec_output_text(value,'stderr') == ''
        assert observed is not None
        vm_id,path,process = observed
        cycle.absent(vm_id,path,process,deadline=deadline)
        assert (set(sessions.iterdir()) if sessions.exists() else set()) == original_paths, 'one-shot left hidden/failed session storage'
    # End-to-end includes provisioning, independent observer fence, command
    # completion and teardown. Raw body clocks are not exec-ready clocks.
    cycle.observe_control('POST /run','vm-fenced-execution-cleanup',request,completed,deadline=deadline)
