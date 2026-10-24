"""Canonical hermetic broker/mock controls; original guest traffic only."""
import shlex
import tempfile
import time
from contextlib import contextmanager
from pathlib import Path

from helpers.content_filter_session import (
    FIXTURE_HOST,
    fixture_config,
    upstream_records,
)
from helpers.mock_server import start_mock_server, stop_process

from .route_lifecycle import validate_exec

UNRELATED_HOST = 'mcp.capsem.test'  # Existing mock DNS fixture, no provider suffix/binding.


class Traffic:
    def __init__(self,cycle,transcript):
        self.cycle,self.transcript = cycle,transcript

    def records(self):
        if not self.transcript.exists():
            return []
        with self.transcript.open('rb') as source:
            raw = source.read(self.cycle.cfg['max_log_bytes']+1)
        assert len(raw) <= self.cycle.cfg['max_log_bytes'], 'broker mock transcript budget exceeded'
        return upstream_records(raw.decode(),required_hosts=(),allowed_hosts=(FIXTURE_HOST,UNRELATED_HOST))

    def __call__(self,vm_id,reference,secret,*,negative,deadline):
        before = len(self.records())
        source = f'''import json, urllib.request, urllib.error
token={reference!r}
request=urllib.request.Request('https://{FIXTURE_HOST}/echo',data=b'route broker proof',headers={{'authorization':'Bearer '+token,'content-type':'text/plain'}})
with urllib.request.urlopen(request,timeout={self.cycle.cfg['exec_timeout_seconds']}) as response:
    echo=json.load(response)
    assert response.status==200 and echo['has_authorization'] is True and echo['authorization_is_broker_ref'] is False
'''
        if negative:
            source = f'''import urllib.request, urllib.error
token={reference!r}
for url,data,headers in [
    ('https://{FIXTURE_HOST}/content/echo',token.encode(),{{'content-type':'text/plain'}}),
    ('https://{UNRELATED_HOST}/echo',b'wrong binding',{{'authorization':'Bearer '+token}})]:
    try:
        urllib.request.urlopen(urllib.request.Request(url,data=data,headers=headers),timeout={self.cycle.cfg['exec_timeout_seconds']})
        raise AssertionError('forbidden broker traffic reached upstream')
    except urllib.error.HTTPError as error:
        assert error.code==403
'''
        source += "\nprint('broker-traffic-verified')\n"
        assert secret not in source, 'raw synthetic secret entered guest command'
        _,result = self.cycle.execute(vm_id,'python3 -c '+shlex.quote(source),deadline=deadline)
        validate_exec(result,'broker-traffic-verified')
        if negative:
            assert len(self.records()) == before, 'forbidden broker request reached mock'
            return
        # Mock logging may complete immediately after the guest reply. Bound
        # observation without accepting extra/duplicate/negative delivery.
        while len(self.records()) <= before:
            if time.monotonic() >= deadline:
                raise TimeoutError('original broker upstream record missing')
            time.sleep(self.cycle.cfg['poll_seconds'])
        (row,) = self.records()[before:]
        assert row['method'] == 'POST' and row['path'] == '/echo'
        assert row['headers']['host'] == FIXTURE_HOST and row['headers']['authorization'] == 'Bearer '+secret
        assert row['request_body'] == 'route broker proof', 'original broker payload differs'


@contextmanager
def credential_target(label,cfg):
    from .route_lifecycle_fixture import lifecycle_target
    with tempfile.TemporaryDirectory(prefix='capsem-route-credentials-') as temporary:
        transcript = Path(temporary)/'upstream.jsonl'
        mock = None
        try:
            mock,ready = start_mock_server(request_log=transcript,dns_answers='routable')
            def prepare(service):
                # Canonical binding only FIXTURE_HOST. Admit unrelated host to
                # the mock so broker binding, not generic network policy, must
                # reject it. No enrollment/vendor request occurs.
                config = fixture_config(ready)+f'''
[network.upstream_overrides."{UNRELATED_HOST}:443"]
dial="{ready['http_addr']}"
protocol="http"
[corp.rules.route_broker_negative_binding]
name="route_broker_negative_binding"
action="allow"
priority=-100
detection_level="informational"
reason="Hermetic unrelated binding negative control."
match='http.host == "{UNRELATED_HOST}" && tcp.port == "443" && http.path == "/echo"'
'''
                (service.home_dir/'corp.toml').write_text(config)
            with lifecycle_target(label,prepare=prepare) as (target,service,reference,certificate):
                # Configure existing guard before boot; no new policy engine.
                transport = target.transport_factory()
                try:
                    from .route_requests import json_request
                    _,value = json_request(transport,target,'PATCH','/plugins/secret_exfiltration/edit',{'mode':'block'},
                        timeout=cfg['timeout_seconds'],maximum=cfg['max_response_bytes'])
                    assert value['id'] == 'secret_exfiltration' and value['config']['mode'] == 'block'
                finally:
                    transport.close()
                yield target,service,reference,certificate,lambda cycle:Traffic(cycle,transcript)
        finally:
            stop_process(mock)
