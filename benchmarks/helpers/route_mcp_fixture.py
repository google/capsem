"""Existing pinned lifecycle/builtin MCP/mock owners, with narrow explicit policy."""
import json
import tempfile
import time
from contextlib import contextmanager
from pathlib import Path
from urllib.parse import urlsplit

from helpers.mock_server import start_mock_server, stop_process
from psutil import Process

from .route_lifecycle import validate_running


class Probe:
    def __init__(self,cycle,transcript):
        self.cycle,self.transcript,self.owner = cycle,transcript,None

    def ready(self,vm_id,*,deadline):
        info = self.cycle.info(vm_id,deadline=deadline)
        validate_running(info,vm_id)
        process = Process(info['pid'])
        identity = (process.pid,process.create_time())
        if self.owner is None:
            self.owner = identity
        assert identity == self.owner and process.is_running(), 'original MCP VM owner retired/replaced'

    def records(self):
        if not self.transcript.exists():
            return []
        with self.transcript.open('rb') as source:
            body = source.read(self.cycle.cfg['max_log_bytes']+1)
        assert len(body) <= self.cycle.cfg['max_log_bytes'], 'MCP transcript budget exceeded'
        rows = [json.loads(line) for line in body.splitlines()]
        assert all('kind' not in row for row in rows), 'loopback builtin call unexpectedly used external DNS'
        return rows

    def before(self):
        return len(self.records())

    def delivered(self,before,url,*,deadline):
        while len(self.records()) <= before:
            if time.monotonic() >= deadline:
                raise TimeoutError('original MCP upstream request missing')
            time.sleep(self.cycle.cfg['poll_seconds'])
        (row,) = self.records()[before:]
        expected = urlsplit(url)
        assert row['method'] == 'GET' and row['path'] == expected.path and row['query'] == expected.query
        assert row['headers']['host'] == expected.netloc
        assert row['request_body'] == '' and row['request_bytes'] == 0


@contextmanager
def mcp_target(label,cfg):
    from .route_lifecycle_fixture import lifecycle_target
    from .route_requests import json_request
    with tempfile.TemporaryDirectory(prefix='capsem-route-mcp-') as temporary:
        transcript = Path(temporary)/'upstream.jsonl'
        mock = None
        try:
            mock,ready = start_mock_server(request_log=transcript)
            base = urlsplit(ready['base_url'])
            assert base.hostname == '127.0.0.1' and base.port is not None
            def prepare(service):
                (service.home_dir/'corp.toml').write_text(f'''
[corp.rules.route_builtin_http]
name="route_builtin_http"
action="allow"
priority=-100
detection_level="informational"
reason="Exact hermetic builtin MCP fixture only."
match='http.host == "127.0.0.1" && tcp.port == "{base.port}" && http.path == "{cfg['path']}" && http.method == "GET"'
''')
            with lifecycle_target(label,prepare=prepare) as (target,service,reference,certificate):
                transport = target.transport_factory()
                try:
                    # Preserve genuine tools/list discovery. default.mcp also
                    # covers that method, so blanket block would deny refresh.
                    for tool,action in (('echo','block'),('grep_http','block'),('http_headers','allow'),('fetch_http','allow')):
                        _,changed = json_request(transport,target,'PATCH',f'/mcp/servers/local/tools/{tool}/edit',{'action':action},
                            timeout=cfg['timeout_seconds'],maximum=cfg['max_response_bytes'])
                        assert changed['server_id'] == 'local' and changed['tool_id'] == tool and changed['action'] == action
                        assert changed['mutation']['status'] == 'applied'
                finally:
                    transport.close()
                yield target,service,reference,certificate,lambda cycle:Probe(cycle,transcript),ready['base_url']
        finally:
            stop_process(mock)
