"""A writable checksum sidecar cannot authorize a modified scanner."""

import hashlib
import io
import platform
import tarfile
import urllib.request
from pathlib import Path
from types import SimpleNamespace

import pytest
from capsem_builder.cache.config import load_paths
from capsem_builder.cache.tools import CachedToolPolicy, ToolDistribution
from capsem_builder.docs_metrics import tools


@pytest.fixture
def scanner(tmp_path, monkeypatch):
    package = io.BytesIO()
    with tarfile.open(fileobj=package, mode="w:gz") as archive:
        member = tarfile.TarInfo("scc")
        member.size = 6
        archive.addfile(member, io.BytesIO(b"binary"))
    data = package.getvalue()
    policy = CachedToolPolicy(
        name="scc",
        version="test",
        cache_stage="toolchain",
        download_timeout_seconds=10,
        distributions=(
            ToolDistribution(
                system=platform.system(),
                machine=platform.machine(),
                url="https://example.test/scanner.tar.gz",
                sha256=hashlib.sha256(b"binary").hexdigest(),
                archive_sha256=hashlib.sha256(data).hexdigest(),
                archive_member="scc",
            ),
        ),
    )
    repo = Path(__file__).resolve().parents[3]
    paths = load_paths(repo, environment={})
    paths = paths.model_copy(update={"repository_root": tmp_path})
    monkeypatch.setattr(tools, "load_paths", lambda _repo: paths)
    monkeypatch.setattr(
        tools, "for_root", lambda _repo: SimpleNamespace(docs_metrics_scanner=policy)
    )
    monkeypatch.setattr(urllib.request, "urlopen", lambda *args, **kwargs: io.BytesIO(data))
    return tmp_path, data


def test_scanner_checks_archive_and_repairs_readonly_cache(scanner):
    repo, _data = scanner
    path = tools.source_scanner(repo)
    assert path.read_bytes() == b"binary"
    assert path.stat().st_mode & 0o777 == 0o555
    assert path.is_relative_to(repo / "cache/target/toolchain")
    path.unlink()
    path.write_bytes(b"tampered")
    path.chmod(0o555)
    path.with_name(path.name + ".sha256").write_text(hashlib.sha256(b"tampered").hexdigest())
    assert tools.source_scanner(repo).read_bytes() == b"binary"


def test_scanner_rejects_modified_archive(scanner, monkeypatch):
    repo, _data = scanner
    monkeypatch.setattr(urllib.request, "urlopen", lambda *args, **kwargs: io.BytesIO(b"tampered"))
    with pytest.raises(ValueError, match="archive checksum mismatch"):
        tools.source_scanner(repo)
