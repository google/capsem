"""Binary/archive routes must return exact independently owned complete bytes."""
from __future__ import annotations

import gzip
from io import BytesIO

import pytest
from helpers.stopped_workspace import VM_ID
from warcio.warcwriter import WARCWriter

from benchmarks.helpers.route_binary import archive_facts, validate_archive, validate_file


def warc(facts: list[dict], *, omit: bool = False, changed: bool = False, summary: bool = True) -> bytes:
    stream = BytesIO()
    writer = WARCWriter(stream, gzip=True, warc_version="1.1")
    def info(which: str, data: bytes):
        return writer.create_warc_record(None, "warcinfo", payload=BytesIO(data), length=len(data), warc_headers_dict={
            "WARC-Record-ID": f"<urn:capsem:{VM_ID}:warcinfo:{which}:fixture>",
            "Content-Type": "application/warc-fields",
        })
    writer.write_record(info("opening", f"capsem-session: {VM_ID}\r\n".encode()))
    for index, fact in enumerate(facts[:-1] if omit else facts):
        key = f"urn:capsem:{VM_ID}:{fact['source_table']}:{fact['event_id']}:{fact['direction']}"
        data = b"changed" if changed and index == 0 else fact["body"]
        record = writer.create_warc_record("https://fixture.invalid/", "resource", payload=BytesIO(data), length=len(data), warc_headers_dict={
            "WARC-Record-ID": f"<{key}>", "Content-Type": fact["content_type"] or "application/octet-stream",
        })
        writer.write_record(record)
    if summary:
        writer.write_record(info("closing", f"capsem-records: {len(facts)}\r\ncapsem-skipped: 0\r\n".encode()))
    return stream.getvalue()


def test_warc_compares_every_original_record_and_requires_complete_framing() -> None:
    facts = archive_facts()
    assert len(facts) == 26 and sum(len(row["body"]) for row in facts) == 188665
    headers = {"content-type": "application/warc", "content-disposition": f'attachment; filename="capsem-session-{VM_ID}.warc.gz"'}
    good = warc(facts)
    validate_archive(good, headers)
    for bad in (warc(facts, omit=True), warc(facts, changed=True), warc(facts, summary=False), good[:-3], good + b"junk"):
        with pytest.raises((AssertionError, ValueError, EOFError, gzip.BadGzipFile)):
            validate_archive(bad, headers)
    with pytest.raises(AssertionError):
        validate_archive(good, {**headers, "content-encoding": "gzip"})


def test_file_validation_checks_live_path_headers_size_and_exact_binary_content() -> None:
    from benchmarks.helpers.route_binary import file_bytes
    data = file_bytes(4096)
    headers = {"content-length": "4096", "content-type": "application/octet-stream", "x-capsem-vm-path": "/workspace/route-4096.bin", "content-disposition": 'attachment; filename="route-4096.bin"'}
    validate_file("/vms/live/files/content?path=route-4096.bin", data, headers)
    for body, changed in ((data[:-1], headers), (b"x" * 4096, headers), (data, {**headers, "x-capsem-vm-path": "/root/route-4096.bin"})):
        with pytest.raises(AssertionError):
            validate_file("/vms/live/files/content?path=route-4096.bin", body, changed)


def test_full_event_bytes_and_capture_vs_transport_flags_are_independent() -> None:
    import copy

    from helpers.stopped_workspace import BODY_EVENT_ID

    from benchmarks.helpers.route_binary import validate_full_event
    facts = [row for row in archive_facts() if row["event_id"] == BODY_EVENT_ID]
    payload = {"event_id": BODY_EVENT_ID, "bodies": [{
        **{key: row[key] for key in ("event_id", "source_table", "direction", "content_type", "original_bytes", "truncated")},
        "stored_bytes": len(row["body"]), "encoding": "utf8", "content": row["body"].decode(),
        "truncated_for_transport": False,
    } for row in facts]}
    validate_full_event(payload)
    for key, value in (("content", "wrong"), ("stored_bytes", 0), ("truncated_for_transport", True), ("encoding", "unknown")):
        changed = copy.deepcopy(payload)
        bodies = changed["bodies"]
        assert isinstance(bodies, list)
        bodies[0][key] = value
        with pytest.raises((AssertionError, ValueError)):
            validate_full_event(changed)


def test_binary_case_counts_preserve_existing_defaults_and_bound_new_profiles() -> None:
    from benchmarks.helpers.route_binary_fixture import expected_file_exports
    from benchmarks.helpers.route_cases import measured_cases
    cases = measured_cases()
    files = [case for case in cases if case.response == "binary"]
    assert len(files) == 2 and all(case.live for case in files)
    warc_case = next(case for case in cases if case.response == "warc")
    assert warc_case.counts(128, 512, 8) == (8, 16, 1), "WARC admits one active export per session"
    assert all(case.counts(128, 512, 8) == (8, 16, 2) for case in files)
    assert all(case.counts(128, 512, 8, 4) == (4, 16, 2) for case in files)
    assert next(case for case in cases if case.path == "/status").counts(128, 512, 8) == (128, 512, 8)
    assert expected_file_exports(cases, samples=128, load_requests=512, concurrency=8, warmup=2, targets=2, override=None) == 148
    assert len({(case.method, case.path) for case in cases}) == 59


def test_binary_fixture_uses_workload_and_preserves_producer_until_drain(monkeypatch, tmp_path) -> None:
    from types import SimpleNamespace

    from benchmarks.helpers import route_binary_fixture as fixture
    calls = []
    def post(path, body, **kwargs):
        calls.append(path)
        assert path == "/vms/live/stop" and body == {}
        return {"success": True, "persistent": True}
    def get(path):
        calls.append(path)
        assert calls[0] == "/vms/live/stop", "read must follow actual producer drain"
        if path.endswith("/enforcement/status"):
            return {"by_rule": [{"rule_id": "profiles.rules.route_binary_export", "count": 148, "rule_action": "allow"}]}
        return {"files": {"actions": [{"action": "export", "count": 148}]}}
    client = SimpleNamespace(post=post, get=get)
    service = SimpleNamespace(client=lambda: client, home_dir=tmp_path)
    observed = []
    monkeypatch.setattr(fixture, "apply_settings_rule", lambda *args, **kwargs: observed.append(kwargs))
    def workload(owner, vm_id, command):
        assert owner is client and vm_id == "live" and "/workspace" in command
        assert "1048576" in command and "4096" in command
        return {"exit_code": 0, "stdout_text": "CAPSEM-BINARY-FIXTURE\n"}
    monkeypatch.setattr(fixture, "workload_exec", workload)
    fixture.seed_binary_files(service, "live")
    assert observed[0]["action"] == "allow" and "route-4096.bin" in observed[0]["match"]
    fixture.finish_binary_files(service, "live", 148)
    with pytest.raises(AssertionError):
        fixture.finish_binary_files(service, "live", 147)
    with pytest.raises(AssertionError):
        fixture.seed_binary_files(service, None)


def test_inflated_archive_and_facts_are_bounded(monkeypatch, tmp_path) -> None:
    from benchmarks.helpers import route_binary as binary
    cfg = binary.corpus_config()
    headers = {"content-type": "application/warc", "content-disposition": f'attachment; filename="capsem-session-{VM_ID}.warc.gz"'}
    bomb = gzip.compress(b" " * (cfg["archive_inflated_max_bytes"] + 1))
    with pytest.raises(AssertionError, match="inflation budget"):
        validate_archive(bomb, headers)
    binary.archive_facts.cache_clear()
    target = tmp_path / "tests/fixtures/session"
    target.mkdir(parents=True)
    (target / "test.facts.msgpack").write_bytes(b"x" * (cfg["archive_facts_max_bytes"] + 1))
    monkeypatch.setattr(binary, "ROOT", tmp_path)
    try:
        with pytest.raises(AssertionError, match="facts byte budget"):
            binary.archive_facts()
    finally:
        binary.archive_facts.cache_clear()
