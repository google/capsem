"""Three finite member operations; link receipts and packets are independent."""
import ipaddress
import re
import time
import tomllib
from functools import partial
from pathlib import Path
from uuid import UUID

from psutil import Process

from .route_lifecycle import Cycle as LifecycleCycle
from .route_measurements import Observations
from .route_requests import json_request
from .stream_measurements import measure, sample_count


def member_config():
    from capsem_builder.gate.benchmarkmemberschema import NetworkMemberConfig
    cfg = tomllib.loads((Path(__file__).resolve().parents[2]/'config/gate.toml').read_text())
    return NetworkMemberConfig.model_validate(cfg['benchmark']['routes']['network_members']).model_dump()


def validate_link_logs(value,addresses,closed,limit):
    events = value['events']
    assert len(events) == 3 and len(events) < limit and value.get('next_cursor') is None, 'link corpus incomplete/duplicated/truncated'
    assert isinstance(value['cursor'],str) and value['cursor']
    assert len({row['sequence'] for row in events}) == 3 and all(type(row['sequence']) is int and row['sequence'] > 0 for row in events)
    assert len({row['event_id'] for row in events}) == 3
    connects,close = {},[]
    sequences = {}
    for row in events:
        assert re.fullmatch(r'[0-9a-f]{12}',row['event_id'])
        assert type(row['timestamp_unix_ms']) is int and row['timestamp_unix_ms'] > 0
        assert str(UUID(row['connection_id'])) == row['connection_id']
        facts = row['event']
        net = facts['network']
        ident = net['source']['vm']['id']
        assert ident in addresses and net['context'] == 'private' and net['protocol'] == 'link', 'not genuine private link history'
        assert net['source'] == net['destination'] == {'vm':{'id':ident},'address':addresses[ident],'port':0}
        if row['event_type'] == 'network.connect':
            assert ident not in connects and facts['decision'] == {'effective':'allow','reason':'linked'}
            connects[ident] = row['connection_id']
            sequences[ident] = row['sequence']
        else:
            assert row['event_type'] == 'network.close' and ident == closed
            assert facts['decision'] == {'effective':'allow','reason':'unlinked'}
            frames = facts['frames']
            assert frames['reason'] == 'Cancelled' and frames['frames_in'] > 0 and frames['frames_out'] > 0
            assert frames['bytes_in'] > frames['frames_in'] and frames['bytes_out'] > frames['frames_out']
            assert set(frames['dropped']) == {'short','source_mac','source_address','unknown','queue_full','storm'}
            assert all(type(n) is int and n >= 0 for n in frames['dropped'].values())
            assert frames['dropped']['source_mac'] == frames['dropped']['source_address'] == 0
            close.append((row['connection_id'],row['sequence']))
    assert set(connects) == set(addresses) and len(close) == 1 and close[0][0] == connects[closed] and close[0][1] > sequences[closed]


class Cycle(LifecycleCycle):
    def __init__(self,transport,target,service,cfg,metrics,probe_factory):
        super().__init__(transport,target,service,cfg,metrics)
        self.probe = probe_factory(self)

    def observe_member(self,operation,request,verify):
        started = time.monotonic()
        deadline = started+self.cfg['timeout_seconds']
        self.probe.deadline = deadline
        receipt,value = request(deadline)
        result = verify(value,deadline)
        assert time.monotonic() <= deadline, 'member effect deadline exceeded'
        observations = Observations()
        observations.append(receipt)
        (case,) = self.cfg['endpoints'][operation]['cases']
        prefix = f'{self.target.label}.{operation} [{case}].'
        for key,metric in observations.metrics(prefix,1).items():
            self.metrics.setdefault(key,{'unit':metric['unit'],'samples':[]})['samples'].extend(metric['samples'])
        self.metrics.setdefault(prefix+'verified_completion_ms',{'unit':'milliseconds','samples':[]})['samples'].append((time.monotonic()-started)*1000)
        return result

    def joined(self,network,vm_id,*,deadline):
        while True:
            info = self.read(f'/networks/{network}',deadline=deadline)
            rows = [row for row in info['members'] if row['vm_id'] == vm_id]
            assert len(rows) == 1, 'attach did not publish original member'
            row = rows[0]
            assert row['state'] != 'failed', 'private cable failed'
            if row['state'] == 'ready':
                subnet = ipaddress.IPv4Network(info['subnet'])
                assert subnet.is_private and subnet.prefixlen == 24
                assert ipaddress.IPv4Address(row['address']) in subnet and row['updated_unix_ms'] > 0
                assert list(self.probe.cables(vm_id).values()) == [row['address']+'/24'], 'guest cable address differs'
                return row['address']
            if time.monotonic() >= deadline:
                raise TimeoutError('private member never became ready')
            time.sleep(self.cfg['poll_seconds'])

    def same_switch(self,original):
        rows = self.probe.switches()
        assert original.is_running() and [(row.pid,row.create_time()) for row in rows] == [(original.pid,original.create_time())], 'original switch identity changed'

    def packets(self,source,address,*,reachable):
        result = self.probe.ping(source,address)
        expected = self.cfg['probe_count'] if reachable else 0
        assert result['sent'] == self.cfg['probe_count'] and result['received'] == expected and result['lost'] == self.cfg['probe_count']-expected, 'independent private reachability differs'


def sample_members(cycle,reference,certificate):
    cfg = cycle.cfg
    deadline = time.monotonic()+cfg['timeout_seconds']
    cycle.probe.deadline = deadline
    assert cycle.listing(deadline=deadline) == [] and cycle.probe.switches() == []
    owners = []
    for name in ('alpha','beta'):
        _,value = cycle.call('POST','/vms/create',{'name':cfg['name']+'-'+name,'persistent':True,
            'cpus':cfg['cpus'],'ram_mb':cfg['ram_mb'],'container':{'image':reference,'registry':{'ca_pem':certificate.read_text()}}},deadline=deadline)
        vm_id = cycle.identify(value)
        cycle.ready(vm_id,reference,deadline=deadline)
        assert cycle.probe.cables(vm_id) == {}, 'fixture member already connected'
        owners.append(vm_id)
    alpha,beta = owners
    _,created = json_request(cycle.transport,cycle.target,'POST','/networks',{'name':cfg['name']},
        timeout=deadline-time.monotonic(),maximum=cfg['max_response_bytes'],expected_status=201)
    network = created['id']
    assert str(UUID(network)) == network and created['members'] == [] and created['name'] == cfg['name']
    cycle.call('PUT',f'/networks/{network}/members/{alpha}',deadline=deadline)
    alpha_address = cycle.joined(network,alpha,deadline=deadline)
    (original,) = cycle.probe.switches()
    def attached(value,deadline):
        assert value['id'] == network and {row['vm_id'] for row in value['members']} == {alpha,beta}
        address = cycle.joined(network,beta,deadline=deadline)
        assert address != alpha_address
        cycle.same_switch(original)
        cycle.packets(alpha,address,reachable=True)
        cycle.packets(beta,alpha_address,reachable=True)
        return address
    beta_address = cycle.observe_member('PUT /networks/{id}/members/{vm_id}',
        lambda deadline:cycle.call('PUT',f'/networks/{network}/members/{beta}',deadline=deadline),attached)
    def detached(value,deadline):
        assert value['id'] == network and [row['vm_id'] for row in value['members']] == [alpha], 'detached member remains'
        assert [row['vm_id'] for row in cycle.read(f'/networks/{network}',deadline=deadline)['members']] == [alpha]
        while cycle.probe.cables(beta):
            if time.monotonic() >= deadline:
                raise TimeoutError('guest retained detached cable')
            time.sleep(cfg['poll_seconds'])
        assert cycle.joined(network,alpha,deadline=deadline) == alpha_address
        cycle.same_switch(original)
        cycle.packets(alpha,beta_address,reachable=False)
    cycle.observe_member('DELETE /networks/{id}/members/{vm_id}',
        lambda deadline:cycle.call('DELETE',f'/networks/{network}/members/{beta}',deadline=deadline),detached)
    addresses = {alpha:alpha_address,beta:beta_address}
    path = f'/networks/{network}/logs?limit={cfg["log_limit"]}'
    # Close is asynchronous switch reporting. Poll its real ledger, never seed
    # events or fabricate a flow record; full independent validation follows.
    deadline = time.monotonic()+cfg['timeout_seconds']
    while len(cycle.read(path,deadline=deadline)['events']) < 3:
        if time.monotonic() >= deadline:
            raise TimeoutError('real link close event missing')
        time.sleep(cfg['poll_seconds'])
    cycle.observe_member('GET /networks/{id}/logs',lambda deadline:cycle.call('GET',path,deadline=deadline),
        lambda value,deadline:validate_link_logs(value,addresses,beta,cfg['log_limit']))
    deadline = time.monotonic()+cfg['timeout_seconds']
    cycle.probe.deadline = deadline
    cycle.call('DELETE',f'/networks/{network}/members/{alpha}',deadline=deadline)
    while original.is_running() or cycle.probe.cables(alpha):
        if time.monotonic() >= deadline:
            raise TimeoutError('last-member switch/cable did not retire')
        time.sleep(cfg['poll_seconds'])
    assert cycle.probe.switches() == []
    _,retired = cycle.call('DELETE',f'/networks/{network}',deadline=deadline)
    assert retired == {'success':True}


def require_member_metrics(metrics,cfg,quick,labels=('service','gateway')):
    count = sample_count(cfg,'cycles',quick)
    for label in labels:
        for operation,entry in cfg['endpoints'].items():
            (case,) = entry['cases']
            for suffix in ('latency_ms','headers_ms','first_body_byte_ms','verified_completion_ms'):
                key = f'{label}.{operation} [{case}].{suffix}'
                if key not in metrics or len(metrics[key]['samples']) != count:
                    raise ValueError('missing or partial member/link samples')
        for index in range(count):
            key = f'{label}.network_members.{index}.wall_s'
            if key not in metrics or len(metrics[key]['samples']) != 1:
                raise ValueError('missing or partial member resources')


def collect_network_members(quick):
    from .route_lifecycle_fixture import lifecycle_target
    from .route_network_member_probe import NetworkProbe
    cfg,metrics = member_config(),{}
    for label in ('service','gateway'):
        for index in range(sample_count(cfg,'cycles',quick)):
            with lifecycle_target(label) as (target,service,reference,certificate):
                transport = target.transport_factory()
                cycle = Cycle(transport,target,service,cfg,metrics,lambda owner,service=service:NetworkProbe(owner,service))
                try:
                    assert service.proc is not None
                    measure(metrics,f'{label}.network_members.{index}',Process(service.proc.pid),partial(sample_members,cycle,reference,certificate))
                    cycle.cleanup()
                    assert cycle.listing() == []
                finally:
                    transport.close()
    require_member_metrics(metrics,cfg,quick)
    return metrics
