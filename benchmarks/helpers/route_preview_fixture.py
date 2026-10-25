"""Reuse Kingslanding's labelled port responder and preview workload owners."""
import json
import tempfile
import time
from contextlib import contextmanager
from pathlib import Path
from types import SimpleNamespace

from helpers.service import ServiceInstance

from .route_measurements import validate_receipt
from .route_requests import service_target


class Client:
    """Adapt the existing fixture to strict original requests, never retry."""
    def __init__(self,cycle,deadline):
        self.cycle,self.deadline = cycle,deadline

    def post(self,path,body):
        value = self.cycle.call('POST',path,body,deadline=self.deadline)[1]
        if path.endswith('/exec'):
            assert value['truncated'] is False, 'preview fixture exec truncated'
        return value

    def upload_file(self,vm_id,name,body):
        assert vm_id in self.cycle.owned and name == 'preview_server.py'
        result = self.cycle.transport.measure_request('POST',f'/vms/{vm_id}/files/content?path={name}',
            body=body,headers={**self.cycle.target.headers,'Content-Type':'application/octet-stream'},
            timeout=self.deadline-time.monotonic(),max_response_bytes=self.cycle.cfg['max_response_bytes'])
        validate_receipt(result)
        assert result.status == 200
        value = json.loads(result.body)
        assert value['success'] is True and value['size'] == len(body)
        assert value['vm_path'] == '/root/'+name
        return value


def start_workload(cycle,vm_id,deadline):
    from tests.ironbank.kingslanding.test_preview import _hits, _start_server
    client = Client(cycle,deadline)
    _start_server(SimpleNamespace(client=lambda:client),{'vm':{'id':vm_id}})
    return lambda:_hits(client,vm_id)


@contextmanager
def preview_target(label,cfg):
    from helpers.settings_policy import write_settings_rule
    registry,grant_exact,image_config = fixture_inputs()
    with (
        tempfile.TemporaryDirectory(prefix='capsem-preview-routes-') as temporary,
        registry(Path(temporary),image_config=image_config) as (reference,certificate,_),
    ):
        service = ServiceInstance(isolated_corp_config=True)
        try:
            grant_exact(service.home_dir,reference,certificate=certificate)
            write_settings_rule(service.home_dir,'route_preview_fixture_import',action='allow',
                detection_level='informational',match='file.import.name == "preview_server.py"',
                reason='Exact isolated canonical synthetic preview server fixture only.')
            service.start()
            yield service_target(service,label),service,reference,certificate,service_target(service,'gateway')
        finally:
            service.stop()


def fixture_inputs():
    from tests.fixtures.oci.registry import grant_exact, registry
    from tests.ironbank.kingslanding.test_workload_surface import IMAGE_CONFIG
    return registry,grant_exact,IMAGE_CONFIG
