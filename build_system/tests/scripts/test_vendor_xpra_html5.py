"""Contracts for the pinned xpra-html5 vendoring script."""

from __future__ import annotations

import hashlib
import importlib.util
import io
import re
import sys
import tarfile
import tomllib
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[3]
SCRIPT = PROJECT_ROOT / "build_system/scripts/build/vendor_xpra_html5.py"
VENDOR = PROJECT_ROOT / "crates/capsem-gateway/vendor/xpra-html5"


def _module():
    spec = importlib.util.spec_from_file_location("vendor_xpra_html5_script", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _ar(members: dict[str, bytes]) -> bytes:
    archive = b"!<arch>\n"
    for name, content in members.items():
        header = f"{name + '/':<16}{0:<12}{0:<6}{0:<6}{'100644':<8}{len(content):<10}`\n"
        archive += header.encode("ascii") + content + (b"\n" if len(content) % 2 else b"")
    return archive


def _deb(files: dict[str, bytes], links: dict[str, str] | None = None) -> bytes:
    data = io.BytesIO()
    with tarfile.open(fileobj=data, mode="w:xz") as tar:
        for name, content in files.items():
            info = tarfile.TarInfo(name)
            info.size = len(content)
            tar.addfile(info, io.BytesIO(content))
        for name, target in (links or {}).items():
            info = tarfile.TarInfo(name)
            info.type = tarfile.SYMTYPE
            info.linkname = target
            tar.addfile(info)
    return _ar({"debian-binary": b"2.0\n", "control.tar.xz": b"", "data.tar.xz": data.getvalue()})


WWW = "./usr/share/xpra/www/"


def test_only_the_client_page_and_what_it_loads_are_taken_unmodified() -> None:
    module = _module()
    package = _deb(
        {
            f"{WWW}index.html": b"<html></html>",
            f"{WWW}index.html.gz": b"gz",
            f"{WWW}js/Client.js": b"class XpraClient {}",
            f"{WWW}js/Client.js.br": b"br",
            f"{WWW}css/client.css": b"body {}",
            f"{WWW}icons/close.png": b"png",
            f"{WWW}connect.html": b"connect",
            f"{WWW}mitm.html": b"mitm",
            f"{WWW}sw.js": b"worker",
            f"{WWW}../../../../etc/escape.js": b"escape",
            "./etc/xpra/html5-client/default-settings.txt": b"settings",
        },
        links={f"{WWW}default-settings.txt": "/etc/xpra/html5-client/default-settings.txt"},
    )
    files = module.client_files(package)
    assert files == {
        "index.html": b"<html></html>",
        "js/Client.js": b"class XpraClient {}",
        "css/client.css": b"body {}",
        "icons/close.png": b"png",
    }
    sums = module.checksums(files)
    assert sums.splitlines()[0] == f"{hashlib.sha256(b'body {}').hexdigest()}  css/client.css"


def test_a_package_without_a_client_or_an_ar_header_is_refused() -> None:
    module = _module()
    with pytest.raises(ValueError, match=r"index\.html"):
        module.client_files(_deb({f"{WWW}js/Client.js": b"x"}))
    with pytest.raises(ValueError, match="ar archive"):
        module.client_files(b"not a deb")


def test_a_package_that_does_not_match_the_pin_is_refused(tmp_path, monkeypatch) -> None:
    module = _module()
    vendor = tmp_path / "vendor"
    vendor.mkdir()
    (vendor / "SOURCE.toml").write_bytes((VENDOR / "SOURCE.toml").read_bytes())
    package = tmp_path / "other.deb"
    package.write_bytes(_deb({f"{WWW}index.html": b"<html></html>"}))
    monkeypatch.setattr(sys, "argv", ["vendor", "--vendor", str(vendor), "--package", str(package)])
    with pytest.raises(SystemExit, match="does not match the pin"):
        module.main()
    assert not (vendor / "www").exists(), "nothing is written from an unpinned package"


def test_the_checked_in_client_is_exactly_what_its_checksums_name() -> None:
    source = tomllib.loads((VENDOR / "SOURCE.toml").read_text(encoding="utf-8"))
    assert source["url"].startswith("https://xpra.org/")
    assert re.fullmatch(r"[0-9a-f]{64}", source["sha256"])
    listed = {}
    for line in (VENDOR / "SHA256SUMS").read_text(encoding="utf-8").splitlines():
        digest, path = line.split("  ", 1)
        listed[path] = digest
    present = {
        str(path.relative_to(VENDOR / "www")): path
        for path in (VENDOR / "www").rglob("*")
        if path.is_file()
    }
    assert set(present) == set(listed)
    for path, file in present.items():
        assert hashlib.sha256(file.read_bytes()).hexdigest() == listed[path], path
