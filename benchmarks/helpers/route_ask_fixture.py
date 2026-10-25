"""Normal pinned image, private HTTP Ask and existing hermetic mock owners."""
import tempfile
from contextlib import contextmanager
from pathlib import Path

from helpers.content_filter_session import FIXTURE_HOST, fixture_config, upstream_records
from helpers.mock_server import start_mock_server, stop_process

RULE = 'route_original_http_ask'
HOST = FIXTURE_HOST


def policy(ready):
    return fixture_config(ready)+f'''
[corp.rules.{RULE}]
name="{RULE}"
action="ask"
priority=-200
detection_level="informational"
reason="One original synthetic HTTP request held for a generation-bound decision."
match='http.host == "{HOST}" && tcp.port == "443" && http.path == "/content/echo"'
'''


def records(path,maximum):
    if not path.exists():
        return []
    with path.open('rb') as source:
        raw = source.read(maximum+1)
    assert len(raw) <= maximum, 'approval upstream transcript exceeds budget'
    return upstream_records(raw.decode(),required_hosts=(),allowed_hosts=(HOST,))


@contextmanager
def ask_target(label,cfg,*,policy_factory=policy):
    from .route_lifecycle_fixture import lifecycle_target
    with tempfile.TemporaryDirectory(prefix='capsem-route-asks-') as temporary:
        transcript = Path(temporary)/'upstream.jsonl'
        mock = None
        try:
            mock,ready = start_mock_server(request_log=transcript,dns_answers='routable')
            def prepare(service):
                (service.home_dir/'corp.toml').write_text(policy_factory(ready))
            with lifecycle_target(label,prepare=prepare) as (target,service,reference,certificate):
                yield target,service,reference,certificate,lambda:records(transcript,cfg['max_log_bytes'])
        finally:
            stop_process(mock)
