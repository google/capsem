"""Finite synthetic broker effects; public counters are not physical ledger proof."""
import json
import re
import stat
import time
import tomllib
from functools import partial
from pathlib import Path

from capsem_builder.crypto import uuid_v4
from psutil import Process

from .route_workload_controls import Cycle as ControlCycle
from .stream_measurements import measure, sample_count

DETAIL = '/plugins/credential_broker/credentials/info'
RELOAD = 'POST /plugins/credential_broker/credentials/reload'


def credential_config():
    from capsem_builder.gate.benchmarkcredentialschema import CredentialRouteConfig
    raw = tomllib.loads((Path(__file__).resolve().parents[2]/'config/gate.toml').read_text())
    return CredentialRouteConfig.model_validate(raw['benchmark']['routes']['credentials']).model_dump()


def reference(secret):
    # Reuse the canonical synthetic store fixture's security-domain BLAKE3
    # reference, not an integrity/dedup hash or a new hashing implementation.
    from tests.ironbank.test_credential_store_lifecycle import _credential_reference
    return _credential_reference('google',secret)


class Cycle(ControlCycle):
    def __init__(self,transport,target,service,cfg,metrics,traffic):
        super().__init__(transport,target,service,cfg,metrics)
        self.traffic = traffic

    def detail(self,count,hydrated,refs,*,deadline):
        while True:
            value = self.read(DETAIL,deadline=deadline)
            store = value['store']
            assert value['plugin_id'] == 'credential_broker'
            assert store['backend'] == 'disk_override' and store['ready'] is True and store['status'] == 'ready' and store['last_error'] is None
            assert store['cached_count'] == count and store['last_hydrated_count'] == hydrated, 'runtime cache/hydration count differs'
            assert type(store['last_hydrated_unix_ms']) is int
            rows = value['inventory']
            if {row['credential_ref'] for row in rows} == set(refs) and len(rows) == len(refs):
                complete = True
                for row in rows:
                    assert row['provider'] == 'google' and 0 <= row['injected_count'] <= 2
                    complete &= row['injected_count'] == 2 and row['observed_count'] >= 1
                    assert row['replay_available'] is refs[row['credential_ref']], 'service memory replay state differs'
                if complete:
                    return value
            if time.monotonic() >= deadline:
                raise TimeoutError('original broker inventory not published')
            time.sleep(self.cfg['poll_seconds'])

    def store(self):
        path = self.service.home_dir/'credential-store.json'
        if not path.exists():
            return {}
        assert path.is_file() and not path.is_symlink() and stat.S_IMODE(path.stat().st_mode) == 0o600
        with path.open('rb') as source:
            raw = source.read(self.cfg['max_store_bytes']+1)
        assert len(raw) <= self.cfg['max_store_bytes'], 'synthetic store budget exceeded'
        return json.loads(raw)


def sample_credentials(cycle,vm_id):
    cfg = cycle.cfg
    refs,persisted = {},{}
    deadline = time.monotonic()+cfg['timeout_seconds']
    cycle.detail(0,0,{},deadline=deadline)
    secrets = {kind:'capsem_test_route_'+kind+'_'+uuid_v4().hex for kind in ('memory','file','reload')}
    for storage in ('memory','file'):
        deadline = time.monotonic()+cfg['timeout_seconds']
        secret,ref = secrets[storage],reference(secrets[storage])
        def verified(value,*,secret=secret,ref=ref,storage=storage,deadline=deadline):
            assert value == {'credential_ref':ref,'storage':storage}
            assert re.fullmatch(r'credential:blake3:[0-9a-f]{64}',ref)
            if storage == 'file':
                persisted['google:'+ref] = secret
            assert cycle.store() == persisted, 'memory persisted or file injection missing'
            cycle.traffic(vm_id,ref,secret,negative=False,deadline=deadline)
            refs[ref] = True
            detail = cycle.detail(len(refs),0,refs,deadline=deadline)
            assert all(raw not in json.dumps(detail) for raw in secrets.values()), 'raw synthetic secret in public inventory'
            return ref
        cycle.observe_control('POST /credentials/inject',storage,
            lambda secret=secret,storage=storage,deadline=deadline:cycle.call('POST','/credentials/inject',{'provider':'google','value':secret,'storage':storage},deadline=deadline),
            verified,deadline=deadline)
    # Canonical lifecycle fixture: add a fresh owned durable account while
    # the service lives. No DB seed. Process first use loads its own cache;
    # service replay_available must stay false until the measured reload.
    secret = secrets['reload']
    ref = reference(secret)
    persisted['google:'+ref] = secret
    path = cycle.service.home_dir/'credential-store.json'
    body = json.dumps(persisted).encode()
    assert len(body) <= cfg['max_store_bytes']
    with path.open('wb') as output:
        output.write(body)
    path.chmod(0o600)
    deadline = time.monotonic()+cfg['timeout_seconds']
    cycle.traffic(vm_id,ref,secret,negative=False,deadline=deadline)
    refs[ref] = False
    before = cycle.detail(2,0,refs,deadline=deadline)
    def hydrated(value):
        assert value['store']['cached_count'] == 3 and value['store']['last_hydrated_count'] == 2
        refs[ref] = True
        after = cycle.detail(3,2,refs,deadline=deadline)
        assert after['store']['last_hydrated_unix_ms'] >= before['store']['last_hydrated_unix_ms']
        assert cycle.store() == persisted
        assert all(raw not in json.dumps(after) and raw not in json.dumps(value) for raw in secrets.values())
    cycle.observe_control(RELOAD,'changed-store',lambda:cycle.call('POST',RELOAD[5:],{},deadline=deadline),hydrated,deadline=deadline)
    # Untimed refusals follow the exact positive inventory snapshots: blocked
    # attempts may themselves be observed by forensic counters, not injections
    # delivered to upstream. Never conflate those semantic counts.
    cycle.traffic(vm_id,reference(secrets['memory']),secrets['memory'],negative=True,
        deadline=time.monotonic()+cfg['timeout_seconds'])


def require_credential_metrics(metrics,cfg,quick,labels=('service','gateway')):
    count = sample_count(cfg,'cycles',quick)
    for label in labels:
        for operation,entry in cfg['endpoints'].items():
            for case in entry['cases']:
                for suffix in ('latency_ms','headers_ms','first_body_byte_ms','verified_completion_ms'):
                    key = f'{label}.{operation} [{case}].{suffix}'
                    if key not in metrics or len(metrics[key]['samples']) != count:
                        raise ValueError('missing or partial credential samples')
        for index in range(count):
            key = f'{label}.credentials.{index}.wall_s'
            if key not in metrics or len(metrics[key]['samples']) != 1:
                raise ValueError('missing or partial credential resources')


def collect_credentials(quick):
    from .route_credential_fixture import credential_target
    cfg,metrics = credential_config(),{}
    for label in ('service','gateway'):
        for index in range(sample_count(cfg,'cycles',quick)):
            with credential_target(label,cfg) as (target,service,reference_image,certificate,probe_factory):
                transport = target.transport_factory()
                cycle = Cycle(transport,target,service,cfg,metrics,None)
                cycle.traffic = probe_factory(cycle)
                try:
                    deadline = time.monotonic()+cfg['timeout_seconds']
                    assert cycle.listing(deadline=deadline) == []
                    _,value = cycle.call('POST','/vms/create',{'name':'route-credentials','persistent':True,
                        'cpus':cfg['cpus'],'ram_mb':cfg['ram_mb'],'container':{'image':reference_image,'registry':{'ca_pem':certificate.read_text()}}},deadline=deadline)
                    vm_id = cycle.identify(value)
                    cycle.ready(vm_id,reference_image,deadline=deadline)
                    assert service.proc is not None
                    measure(metrics,f'{label}.credentials.{index}',Process(service.proc.pid),partial(sample_credentials,cycle,vm_id))
                    cycle.cleanup()  # original producer drain, physical joins remain Ironbank-owned
                    assert cycle.listing() == []
                finally:
                    transport.close()
    require_credential_metrics(metrics,cfg,quick)
    return metrics
