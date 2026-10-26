import hashlib
import io
import tarfile

import pytest
from capsem_builder.docs_metrics import tools


def test_scanner_checks_archive_and_repairs_readonly_cache(tmp_path, monkeypatch):
    package = io.BytesIO()
    with tarfile.open(fileobj=package, mode="w:gz") as archive:
        member = tarfile.TarInfo("scc")
        member.size = 6
        archive.addfile(member, io.BytesIO(b"binary"))
    data = package.getvalue()
    key = (tools.platform.system(), tools.platform.machine())
    monkeypatch.setitem(tools.ARCHIVES, key, ("scanner.tar.gz", hashlib.sha256(data).hexdigest()))
    monkeypatch.setattr(tools.urllib.request, "urlopen", lambda *args, **kwargs: io.BytesIO(data))
    path = tools.source_scanner(tmp_path)
    assert path.read_bytes() == b"binary"
    assert path.stat().st_mode & 0o777 == 0o555
    path.with_name(path.name + ".sha256").write_text("invalid")
    assert tools.source_scanner(tmp_path).read_bytes() == b"binary"
    monkeypatch.setitem(tools.ARCHIVES, key, ("scanner.tar.gz", "invalid"))
    path.with_name(path.name + ".sha256").write_text("invalid")
    with pytest.raises(ValueError, match="SHA-256"):
        tools.source_scanner(tmp_path)
