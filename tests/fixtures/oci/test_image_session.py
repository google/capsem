"""Fixture assembly preserves prepared-root discovery; native proof is separate."""

from contextlib import contextmanager
from pathlib import Path
from types import SimpleNamespace

import pytest
from helpers import image_session


@pytest.mark.parametrize("prepared", ["rootfs-v2", "rootfs", None])
def test_session_fixture_selects_catalog_for_each_prepared_layout(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, prepared: str | None,
) -> None:
    home, layout = tmp_path / "home", tmp_path / "layout"
    home.mkdir()
    layout.mkdir()
    if prepared is not None:
        (layout / prepared).mkdir()
    certificate = tmp_path / "registry.pem"
    certificate.write_text("fixture certificate")
    reference = "localhost:3713/library/debug@sha256:" + "a" * 64
    grants = []

    @contextmanager
    def registry(*_args):
        yield reference, certificate, []

    monkeypatch.setattr(image_session, "layout_registry", registry)
    monkeypatch.setattr(image_session, "grant_exact", lambda *args, **kwargs: grants.append((args, kwargs)))
    monkeypatch.setattr(image_session, "wait_running", lambda *_args, **_kwargs: None)
    client = SimpleNamespace(post=lambda *_args, **_kwargs: {"id": "fixture-session"},
                             delete=lambda *_args, **_kwargs: {"success": True})
    service = SimpleNamespace(home_dir=home, client=lambda: client)
    with image_session.session_of(service, tmp_path / "registry", "fixture", layout=layout, image_name="debug") as identity:
        assert identity == "fixture-session"
    assert len(grants) == 1
    _, options = grants[0]
    assert options["catalog"] == ("localhost:3713/library/catalog:stable" if prepared else None)
    assert options["certificate"] == certificate
