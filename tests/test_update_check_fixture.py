"""Source-only update fixture contracts; never a native updater execution."""
import json
import os
import urllib.request

import pytest
from helpers import service as service_owner
from helpers.update_release import (
    _copy_user_dir_capsem,
    _serve_manifest_release,
    _write_manifest_metadata,
)


def test_original_manifest_server_records_exact_request_and_closes(tmp_path):
    body = b'{"known":"release-graph"}'
    with _serve_manifest_release({'/manifest.json':body}) as (base,requests):
        with urllib.request.urlopen(base+'/manifest.json',timeout=2) as response:
            assert response.read() == body
        assert requests == ['/manifest.json']
        _write_manifest_metadata(tmp_path,base+'/manifest.json')
        metadata = json.loads((tmp_path/'assets/manifest-metadata.json').read_bytes())
        assert metadata['manifest_url'] == base+'/manifest.json'
        assert metadata['origin'] == 'package'
    assert requests == ['/manifest.json']


def test_original_userdir_copy_is_distinct_owned_bytes(tmp_path):
    source = tmp_path/'source'
    source.write_bytes(b'source-only-not-a-real-CLI')
    private = _copy_user_dir_capsem(source,tmp_path/'home')
    assert private == tmp_path/'home/bin/capsem'
    assert private.read_bytes() == source.read_bytes() and not private.samefile(source)
    assert private.stat().st_mode & 0o777 == 0o755


def test_prepared_cli_binding_fails_before_allocating_fixture(tmp_path, monkeypatch):
    monkeypatch.delenv('CAPSEM_RELEASE_BIN_DIR',raising=False)
    monkeypatch.setattr(service_owner,'make_service_home_run_dirs',lambda:pytest.fail('allocated before preflight'))
    with pytest.raises(RuntimeError,match='prepared CLI'):
        service_owner.ServiceInstance(update_cli=tmp_path/'capsem')


def test_real_cli_binding_is_private_and_never_global_env(tmp_path, monkeypatch):
    source = tmp_path/'frozen/capsem'
    source.parent.mkdir()
    source.write_bytes(b'EXPLICIT SOURCE DOUBLE, never executable in this test')
    source.chmod(0o755)
    monkeypatch.setenv('CAPSEM_RELEASE_BIN_DIR',str(source.parent))
    monkeypatch.setenv('CAPSEM_CLI','ambient-must-not-change')
    service = service_owner.ServiceInstance(update_cli=source,sign_binaries=False)
    try:
        assert service.update_cli == service.home_dir/'bin/capsem'
        assert service.update_cli.read_bytes() == source.read_bytes()
        assert not service.update_cli.samefile(source)
        assert os.environ['CAPSEM_CLI'] == 'ambient-must-not-change'
    finally:
        service.stop()


@pytest.mark.parametrize('fault',['missing','foreign','symlink','not-executable'])
def test_bad_selected_cli_never_allocates_a_service(tmp_path,monkeypatch,fault):
    root = tmp_path/'frozen'
    root.mkdir()
    source = root/'capsem'
    if fault != 'missing':
        source.write_bytes(b'source double')
        source.chmod(0o644 if fault == 'not-executable' else 0o755)
    if fault == 'symlink':
        target = root/'target'
        source.rename(target)
        source.symlink_to(target)
    monkeypatch.setenv('CAPSEM_RELEASE_BIN_DIR',str(root))
    monkeypatch.setattr(service_owner,'make_service_home_run_dirs',lambda:pytest.fail('bad CLI allocated service'))
    with pytest.raises(RuntimeError,match='prepared CLI'):
        service_owner.ServiceInstance(update_cli=tmp_path/'foreign' if fault == 'foreign' else source)


def test_private_cli_change_fails_before_signing_or_launch(tmp_path,monkeypatch):
    source = tmp_path/'frozen/capsem'
    source.parent.mkdir()
    source.write_bytes(b'source double')
    source.chmod(0o755)
    monkeypatch.setenv('CAPSEM_RELEASE_BIN_DIR',str(source.parent))
    service = service_owner.ServiceInstance(update_cli=source)
    monkeypatch.setattr(service_owner,'sign_binary',lambda _:pytest.fail('signed before private binding validation'))
    try:
        assert service.update_cli is not None
        service.update_cli.unlink()
        service.update_cli.symlink_to(source)
        with pytest.raises(RuntimeError,match='private prepared CLI'):
            service.start()
    finally:
        service.stop()


def test_private_cli_only_changes_the_owned_child_environment(tmp_path,monkeypatch):
    source = tmp_path/'frozen/capsem'
    source.parent.mkdir()
    source.write_bytes(b'source double')
    source.chmod(0o755)
    monkeypatch.setenv('CAPSEM_RELEASE_BIN_DIR',str(source.parent))
    monkeypatch.setenv('CAPSEM_CLI','ambient')
    monkeypatch.setenv('CAPSEM_RELEASE_MANIFEST_URL','ambient-url')
    service = service_owner.ServiceInstance(update_cli=source,sign_binaries=False)
    calls = []
    class EndBeforeChild(Exception):
        pass
    def launch(*args,**kwargs):
        calls.append(kwargs['env'])
        raise EndBeforeChild
    monkeypatch.setattr(service_owner.subprocess,'Popen',launch)
    try:
        with pytest.raises(EndBeforeChild):
            service.start()
        assert calls[0]['CAPSEM_CLI'] == str(service.update_cli)
        assert 'CAPSEM_RELEASE_MANIFEST_URL' not in calls[0]
        assert calls[0]['CAPSEM_HOME'] == str(service.home_dir)
        assert os.environ['CAPSEM_CLI'] == 'ambient' and os.environ['CAPSEM_RELEASE_MANIFEST_URL'] == 'ambient-url'
        assert service.proc is None
    finally:
        service.stop()


def test_failed_private_copy_cleans_only_its_owned_home(tmp_path,monkeypatch):
    from helpers import update_release
    source = tmp_path/'frozen/capsem'
    source.parent.mkdir()
    source.write_bytes(b'source double')
    source.chmod(0o755)
    monkeypatch.setenv('CAPSEM_RELEASE_BIN_DIR',str(source.parent))
    home,run = tmp_path/'owned',tmp_path/'owned/run'
    run.mkdir(parents=True)
    monkeypatch.setattr(service_owner,'make_service_home_run_dirs',lambda:(home,run))
    def torn(*args):
        raise OSError('original private copy failed')
    monkeypatch.setattr(update_release,'_copy_user_dir_capsem',torn)
    with pytest.raises(OSError,match='copy failed'):
        service_owner.ServiceInstance(update_cli=source)
    assert not home.exists() and source.read_bytes() == b'source double'
