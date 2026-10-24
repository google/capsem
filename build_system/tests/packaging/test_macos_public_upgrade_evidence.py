"""Fail-closed evidence from labelled disposable filesystem/HTTP doubles."""

from __future__ import annotations

import json
from pathlib import Path
from types import SimpleNamespace

import pytest
from capsem_builder.release.tools import release_macos_apply, release_macos_pairing
from capsem_builder.release.tools import release_macos_public_upgrade as public

from build_system.tests.packaging.test_macos_binary_apply import inputs as inputs
from build_system.tests.packaging.test_macos_binary_apply_evidence import evidence
from build_system.tests.packaging.test_macos_public_upgrade import public_pair


def test_zero_before_vm_requires_actual_empty_original_list():
    class Owner:
        def measure_request(self, method, path, **limits):
            assert (method, path) == ("GET", "/vms/list")
            assert limits == {"timeout": 1, "max_response_bytes": 1024}
            return SimpleNamespace(status=200, body=b'{"sandboxes":[]}')

    public.require_empty_vms(Owner(), timeout=1, maximum=1024)


@pytest.mark.parametrize(
    "body,status", [(b"{}", 200), (b'{"sandboxes":[{}]}', 200), (b'{"sandboxes":[]}', 404)]
)
def test_empty_noop_or_error_never_proves_original_vm_state(body, status):
    owner = SimpleNamespace(
        measure_request=lambda *a, **kw: SimpleNamespace(status=status, body=body)
    )
    with pytest.raises(ValueError):
        public.require_empty_vms(owner, timeout=1, maximum=1024)


def test_inert_original_file_family_preserved_without_parsing_db(tmp_path):
    sessions = tmp_path / "sessions"
    sessions.mkdir()
    (sessions / "main.db").write_bytes(b"labelled old-format bytes; never opened as SQLite")
    (sessions / "main.db-wal").write_bytes(b"original WAL payload")
    before = public.original_files(tmp_path, maximum=1024)
    assert set(before) == {"main.db", "main.db-wal"}
    assert public.original_files(tmp_path, maximum=1024) == before
    (sessions / "main.db").write_bytes(b"modified")
    assert public.original_files(tmp_path, maximum=1024) != before


@pytest.mark.parametrize("damage", ["missing", "symlink", "oversize", "directory"])
def test_original_files_fail_closed(tmp_path, damage):
    root = tmp_path / "sessions"
    root.mkdir()
    path = root / "main.db"
    if damage == "symlink":
        path.symlink_to(tmp_path / "other")
    elif damage == "oversize":
        path.write_bytes(b"x" * 1025)
    elif damage == "directory":
        path.mkdir()
    with pytest.raises((ValueError, OSError)):
        public.original_files(tmp_path, maximum=1024)


def test_source_selection_joins_original_cli_metadata_and_manifest(tmp_path):
    manifest = tmp_path / "manifest.json"
    manifest.write_text('{"channel":"stable"}')
    spec = {
        "channel": "stable",
        "before": {
            "manifest_sha256": public._sha(manifest),
            "metadata_manifest_url": "https://release.capsem.org/assets/stable/manifest.json",
        },
    }
    metadata = {
        "manifest_url": "http://127.0.0.1:18765/candidate/assets/stable/manifest.json",
        "origin": "update",
        "channel": "stable",
        "channel_locked": False,
    }
    result = public.feed_selection(spec, metadata, manifest, source=metadata["manifest_url"])
    assert result["original_embedded_url"] != result["selected_feed"]
    for key, value in (
        ("origin", "package"),
        ("channel", "corp"),
        ("manifest_url", "https://other.test"),
        ("channel_locked", True),
    ):
        with pytest.raises(ValueError):
            public.feed_selection(
                spec, {**metadata, key: value}, manifest, source=metadata["manifest_url"]
            )


def test_public_activation_requires_complete_original_http():
    valid = {"outcome": "succeeded", "http_status": 200, "attempts": 1, "transport": "gateway_http"}
    public.require_original_success(valid)
    for damage in (
        {"outcome": "transport_failure"},
        {"http_status": 500},
        {"attempts": 2},
        {"transport": "service_uds"},
    ):
        with pytest.raises(ValueError):
            public.require_original_success({**valid, **damage})


def test_concurrent_original_writer_cannot_supply_a_torn_snapshot(tmp_path, monkeypatch):
    sessions = tmp_path / "sessions"
    sessions.mkdir()
    path = sessions / "main.db"
    path.write_bytes(b"original")
    original = public.file_identity

    def change(file):
        result = original(file)
        file.write_bytes(b"changed-same-or-different-size")
        return result

    monkeypatch.setattr(public, "file_identity", change)
    with pytest.raises(ValueError, match="changed during"):
        public.original_files(tmp_path, maximum=1024)


def full_public_evidence(tmp_path, inputs):
    normal = evidence(tmp_path, inputs)
    before, after = public_pair(inputs)
    spec = release_macos_pairing.validate_public_pair(before=before, after=after, channel="nightly")
    spec.update(inputs=normal["inputs"], limits=normal["limits"])
    path = tmp_path / "binary-apply-request.json"
    request = json.loads(path.read_text())
    request.update(outcome="succeeded", http_status=200, transport="gateway_http")
    request.pop("error_type")
    path.write_text(json.dumps(request))
    path = tmp_path / "binary-apply-transition.json"
    transition = json.loads(path.read_text())
    transition["kind"] = "public_predecessor_upgrade"
    path.write_text(json.dumps(transition))
    source = request["source"]
    original = {
        "scope": "zero_prior_vm_public_upgrade",
        "zero_before_vms": True,
        "original_files_preserved": True,
        "files": {"main.db": [1, 2, 3, "f" * 64]},
        "feed_selection": {
            "selected_feed": source,
            "original_embedded_url": spec["before"]["metadata_manifest_url"],
            "before_manifest_sha256": spec["before"]["manifest_sha256"],
            "selection": "original_cli_update_assets_manifest",
        },
        "content_domains": {
            phase: spec[phase]["content_identity"] for phase in ("before", "after")
        },
    }
    (tmp_path / "public-upgrade-state.json").write_text(json.dumps(original))
    return spec, source


def test_full_scoped_report_requires_http_and_tagged_original_preservation(
    inputs, tmp_path, monkeypatch
):
    spec, source = full_public_evidence(tmp_path, inputs)
    report = release_macos_apply.finish(tmp_path, spec, source=source, maximum=16384)
    assert report["scope"] == "public_predecessor_upgrade"
    assert report["public_predecessor"]["content_domains"]["before"]["domain"] == "legacy_profiles"
    assert report["capabilities"]["physical_vz_boot"] is False
    root = Path(__file__).resolve().parents[3]
    monkeypatch.syspath_prepend(str(root / "build_system/packaging/macos"))
    import macos_binary_apply

    path = tmp_path / "report.json"
    path.write_text(json.dumps(report))
    assert macos_binary_apply.validate_report(path, spec, source=source) == report


@pytest.mark.parametrize(
    "damage",
    [
        "eof",
        "retry",
        "vms",
        "preservation",
        "domain",
        "feed",
        "files",
        "foreign_file",
        "file_identity",
    ],
)
def test_public_report_rejects_native_success_with_missing_original_effect(
    inputs, tmp_path, damage
):
    spec, source = full_public_evidence(tmp_path, inputs)
    path = tmp_path / (
        "binary-apply-request.json" if damage in {"eof", "retry"} else "public-upgrade-state.json"
    )
    value = json.loads(path.read_text())
    if damage == "eof":
        value.update(outcome="transport_failure", http_status=None)
    elif damage == "retry":
        value["attempts"] = 2
    elif damage == "vms":
        value["zero_before_vms"] = False
    elif damage == "preservation":
        value["original_files_preserved"] = False
    elif damage == "domain":
        value["content_domains"]["before"] = spec["after"]["content_identity"]
    elif damage == "feed":
        value["feed_selection"]["before_manifest_sha256"] = "a" * 64
    elif damage == "foreign_file":
        value["files"]["main.bodies/../outside"] = [1, 2, 3, "f" * 64]
    elif damage == "file_identity":
        value["files"]["main.db"] = True
    else:
        value["files"] = {}
    path.write_text(json.dumps(value))
    with pytest.raises(ValueError):
        release_macos_apply.finish(tmp_path, spec, source=source, maximum=16384)


def test_public_state_cli_captures_original_then_rejects_mutation(tmp_path, monkeypatch):
    from helpers import http_transport

    root = tmp_path / "share"
    root.mkdir()
    home = tmp_path / "home"
    (home / "sessions").mkdir(parents=True)
    (home / "assets").mkdir()
    (home / "sessions/main.db").write_bytes(b"old format remains opaque")
    manifest = home / "assets/manifest.json"
    manifest.write_text('{"channel":"stable"}')
    source = "http://127.0.0.1:18765/candidate/assets/stable/manifest.json"
    spec = {
        "scope": "public_predecessor_upgrade",
        "channel": "stable",
        "before": {
            "manifest_sha256": public._sha(manifest),
            "metadata_manifest_url": "https://release.capsem.org/assets/stable/manifest.json",
            "content_identity": {"domain": "legacy_profiles", "sha256": "a" * 64},
        },
        "after": {"content_identity": {"domain": "runtime", "sha256": "b" * 64}},
        "limits": {"timeout": 1, "maximum": 16384},
    }
    (root / "binary-apply-pair.json").write_text(json.dumps(spec))
    (home / "assets/manifest-metadata.json").write_text(
        json.dumps(
            {
                "manifest_url": source,
                "origin": "update",
                "channel": "stable",
                "channel_locked": False,
            }
        )
    )

    class Owner:
        def __init__(self, **kwargs):
            assert kwargs["socket_path"] == str(home / "run/service.sock")

        def measure_request(self, *args, **kwargs):
            return SimpleNamespace(status=200, body=b'{"sandboxes":[]}')

        def close(self):
            pass

    monkeypatch.setattr(http_transport, "Transport", Owner)
    argv = ["public-owner", "capture", "--root", str(root), "--home", str(home), "--source", source]
    monkeypatch.setattr("sys.argv", argv)
    assert public.main() == 0
    original = json.loads((root / "public-upgrade-state.json").read_text())
    assert original["zero_before_vms"] is True and "main.db" in original["files"]
    argv[1] = "preserved"
    assert public.main() == 0
    (home / "sessions/main.db").write_bytes(b"unexpected reset")
    with pytest.raises(ValueError, match="changed during public"):
        public.main()
    observed = json.loads((root / "public-upgrade-observed-files.json").read_text())
    assert observed["original_files_preserved"] is False
    assert observed["files"] != original["files"]
    assert (
        json.loads((root / "public-upgrade-state.json").read_text())["files"] == original["files"]
    )
