"""Existing Kingslanding cable/switch/protocol owners behind strict gateway IO."""
import math
import time
from types import SimpleNamespace

from helpers.service import exec_output_text


class NetworkProbe:
    def __init__(self,cycle,service):
        from tests.ironbank.kingslanding import network
        from tests.ironbank.test_managed_network_lifecycle import switches
        self.cycle,self.service,self.network,self.switch_owners = cycle,service,network,switches
        self.deadline = time.monotonic()+cycle.cfg['timeout_seconds']
        self.bridge = SimpleNamespace(client=lambda:SimpleNamespace(post=self.post))

    def post(self,path,body,*,timeout):
        # Canonical helpers own packet commands and byte parsing. Only adapt
        # their public HTTP client to the existing single-attempt IO owner.
        remaining = self.deadline-time.monotonic()
        if remaining <= 0:
            raise TimeoutError('private probe deadline expired')
        assert body['target'] == 'vm', 'canonical cable probe must target VM'
        body = {**body,'timeout_secs':min(body['timeout_secs'],self.cycle.cfg['exec_timeout_seconds'],math.ceil(remaining))}
        _,value = self.cycle.call('POST',path,body,deadline=min(self.deadline,time.monotonic()+timeout))
        assert value['exit_code'] == 0 and value['truncated'] is False
        assert exec_output_text(value,'stderr') == '', 'private probe emitted stderr'
        return value

    def cables(self,vm_id):
        return self.network.cables(self.bridge,{'id':vm_id})

    def ping(self,source,address):
        cfg = self.cycle.cfg
        return self.network.ping(self.bridge,{'id':source},address,cfg['probe_count'],
            interval_ms=cfg['probe_interval_ms'],timeout_ms=cfg['probe_timeout_ms'])

    def switches(self):
        return self.switch_owners(self.service)
