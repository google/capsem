"""Actual isolated corporate reload and original HTTP block/restore effects."""
import shlex
import time
import tomllib
from functools import partial
from pathlib import Path
from uuid import uuid4

from helpers.content_filter_session import fixture_config
from psutil import Process

from .route_ask_fixture import HOST, ask_target
from .route_credentials import require_credential_metrics
from .route_lifecycle import validate_exec
from .route_mcp_fixture import Probe as OwnerProbe
from .route_policy import FileSnapshot
from .route_save import generation
from .route_workload_controls import Cycle
from .stream_measurements import measure, sample_count

OPERATION = 'POST /corp/reload'
RULE = 'route_original_reload_block'
BLOCK = f'''
[corp.rules.{RULE}]
name="{RULE}"
action="block"
priority=-200
detection_level="informational"
reason="Block only the original isolated synthetic HTTP flow after reload."
match='http.host == "{HOST}" && tcp.port == "443" && http.path == "/content/echo"'
'''.encode()


def reload_config():
    from capsem_builder.gate.benchmarkreloadschema import ReloadRouteConfig
    raw = tomllib.loads((Path(__file__).resolve().parents[2]/'config/gate.toml').read_text())
    return ReloadRouteConfig.model_validate(raw['benchmark']['routes']['reload_controls']).model_dump()


def active_policy(cycle,vm_id,deadline):
    path = cycle.session(vm_id,deadline=deadline)/'vm'/'active_policy.toml'
    assert not path.is_symlink() and path.is_file(), 'active policy not original regular artifact'
    with path.open('rb') as source:
        raw = source.read(cycle.cfg['max_policy_bytes']+1)
    assert 0 < len(raw) <= cycle.cfg['max_policy_bytes'], 'active policy exceeds budget'
    return raw,tomllib.loads(raw.decode())


class Probe(OwnerProbe):
    def __init__(self,cycle,records):
        super().__init__(cycle,None)
        self.upstream = records

    def __call__(self,vm_id,*,allowed,deadline):
        before = len(self.upstream())
        marker = 'capsem-original-reload-'+uuid4().hex
        bound = self.cycle.cfg['max_response_bytes']
        source = f'''import urllib.request,urllib.error
marker={marker!r}
request=urllib.request.Request('https://{HOST}/content/echo',data=marker.encode(),headers={{'content-type':'text/plain'}})
try:
    response=urllib.request.urlopen(request,timeout={self.cycle.cfg['exec_timeout_seconds']})
except urllib.error.HTTPError as error:
    response=error
with response:
    body=response.read({bound}+1).decode()
    assert len(body.encode()) <= {bound}
    assert response.status == {200 if allowed else 403}
    assert (body == marker) if {allowed!r} else (marker not in body)
print('reload-http-effect-verified')
'''
        _,value = self.cycle.execute(vm_id,'python3 -c '+shlex.quote(source),deadline=deadline)
        validate_exec(value,'reload-http-effect-verified')
        if not allowed:
            assert len(self.upstream()) == before, 'blocked original reached upstream'
            return
        while len(self.upstream()) <= before:
            assert time.monotonic() < deadline, 'allowed original upstream request missing'
            time.sleep(self.cycle.cfg['poll_seconds'])
        rows = self.upstream()[before:]
        assert len(rows) == 1, 'original HTTP request replayed'
        (row,) = rows
        assert row['method'] == 'POST' and row['path'] == '/content/echo'
        assert row['headers']['host'] == HOST and row['request_body'] == marker


def sample_reload(cycle,reference,certificate,probe):
    cfg,deadline = cycle.cfg,time.monotonic()+cycle.cfg['timeout_seconds']
    snapshot = FileSnapshot(cycle.service.home_dir,cfg['max_policy_bytes'])
    corp = snapshot.files['corp.toml']
    assert corp is not None and snapshot.files['corp-source.json'] is None, 'reload fixture must have only local original policy'
    original = corp[0]
    assert cycle.listing(deadline=deadline) == []
    try:
        _,created = cycle.call('POST','/vms/create',{'name':cfg['name'],'cpus':cfg['cpus'],'ram_mb':cfg['ram_mb'],
            'container':{'image':reference,'registry':{'ca_pem':certificate.read_text()}}},deadline=deadline)
        vm_id = cycle.identify(created)
        cycle.ready(vm_id,reference,deadline=deadline)
        probe.ready(vm_id,deadline=deadline)
        original_generation = generation(cycle,vm_id,deadline)
        baseline,parsed = active_policy(cycle,vm_id,deadline)
        assert RULE not in parsed['corp_rules']['corp']['rules']
        probe(vm_id,allowed=True,deadline=deadline)
        for blocked,case in ((True,'block-original'),(False,'restore-original')):
            if blocked:
                assert len(original)+len(BLOCK) <= cfg['max_policy_bytes']
                (cycle.service.home_dir/'corp.toml').write_bytes(original+BLOCK)
            else:
                snapshot.restore()
            def verify(value,*,blocked=blocked):
                assert value.get('success') is True and type(value.get('reloaded')) is int and value['reloaded'] == 1, 'reload did not acknowledge exactly one original owner'
                probe.ready(vm_id,deadline=deadline)
                assert generation(cycle,vm_id,deadline) == original_generation, 'reload replaced original publication owner'
                raw,active = active_policy(cycle,vm_id,deadline)
                if blocked:
                    assert RULE in active['corp_rules']['corp']['rules'], 'reload left original policy unchanged'
                    assert active['corp_rules']['corp']['rules'][RULE]['action'] == 'block'
                    assert (cycle.service.home_dir/'corp.toml').read_bytes() == original+BLOCK
                else:
                    assert raw == baseline, 'reload did not restore exact original active policy'
                    snapshot.assert_exact()
                probe(vm_id,allowed=not blocked,deadline=deadline)
            cycle.observe_control(OPERATION,case,
                lambda:cycle.call('POST','/corp/reload',{},deadline=deadline),verify,deadline=deadline)
        cycle.cleanup()
        assert cycle.listing(deadline=deadline) == []
    finally:
        # On failure the owning service fixture stops any remaining process;
        # restoring isolated input files is not a claim of effective reload.
        snapshot.restore()


def require_reload_metrics(metrics,cfg,quick,labels=('service','gateway')):
    require_credential_metrics(metrics,cfg,quick,labels,resource_domain='reload_controls')


def collect_reload_controls(quick):
    cfg,metrics = reload_config(),{}
    for label in ('service','gateway'):
        for index in range(sample_count(cfg,'cycles',quick)):
            with ask_target(label,cfg,policy_factory=fixture_config) as (target,service,reference,certificate,records):
                transport = target.transport_factory()
                cycle = Cycle(transport,target,service,cfg,metrics)
                try:
                    assert service.proc is not None
                    measure(metrics,f'{label}.reload_controls.{index}',Process(service.proc.pid),
                        partial(sample_reload,cycle,reference,certificate,Probe(cycle,records)))
                finally:
                    transport.close()
    require_reload_metrics(metrics,cfg,quick)
    return metrics
