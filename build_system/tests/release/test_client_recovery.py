"""Client continuation cannot carry failed native qualification into publication."""

from copy import deepcopy

import pytest
from capsem_builder.release.tools.verify_release_recovery_run import verify_client_recovery_run

SOURCE = "a" * 40
NAMES = [
    "source-identity", "client-publication-prerequisites",
    "runtime-preflight / Verify the public runtime graph", "fast-gate / static",
    "Resolve serialized channel source manifest", "preflight",
    "build-app-linux (arm64, ubuntu-24.04-arm)", "build-app-linux (x86_64, ubuntu-24.04)",
    "build-app-macos", "Author binary candidate source manifest", "Complete binary pairing gate",
    "Install exact candidate macOS package", "Install exact candidate Linux package (arm64)",
    "Install exact candidate Linux package (x86_64)", "create-release", "assemble-release-channel",
    "verify-release-candidate", "deploy-release-channel / Publish generated channel", "verify-release-downloads",
]


def run_record():
    return {"databaseId": 123, "headSha": SOURCE, "headBranch": f"capsem-source-{SOURCE}",
            "workflowName": "Release", "event": "workflow_dispatch", "status": "completed", "conclusion": "failure",
            "jobs": [{"name": name, "conclusion": "success"} for name in NAMES]
                    + [{"name": "publish-client-packages / publish", "conclusion": "failure"},
                       {"name": "cleanup-unpublished-version-claim", "conclusion": "skipped"}]}


def test_only_registry_failure_can_resume_clients():
    verify_client_recovery_run(run_record(), "123", SOURCE)


@pytest.mark.parametrize("name", NAMES)
def test_every_native_proof_is_required(name):
    record = deepcopy(run_record())
    for job in record["jobs"]:
        if job["name"] == name:
            job["conclusion"] = "failure"
    with pytest.raises(ValueError):
        verify_client_recovery_run(record, "123", SOURCE)


@pytest.mark.parametrize("field,value", [("headSha", "b" * 40), ("headBranch", "main"),
                                        ("event", "pull_request"), ("workflowName", "CI"),
                                        ("databaseId", 124), ("status", "in_progress")])
def test_recovery_binds_original_run_identity(field, value):
    record = run_record()
    record[field] = value
    with pytest.raises(ValueError):
        verify_client_recovery_run(record, "123", SOURCE)


def test_native_consumer_fixture_must_match_public_bytes(tmp_path):
    from capsem_builder.release.tools.client_recovery import verify_native
    from capsem_builder.release.tools.verify_immutable_publication import file_identity

    package = tmp_path / "candidate.deb"
    package.write_bytes(b"qualified native fixture")
    size, digest = file_identity(package)
    manifest = {"channel": "stable", "packages": [{"name": package.name, "source_commit": SOURCE,
                "platform": "linux", "bytes": size, "digest": {"sha256": digest}}]}
    verify_native(package, manifest, SOURCE)
    package.write_bytes(b"unqualified fixture")
    with pytest.raises(ValueError, match="differs"):
        verify_native(package, manifest, SOURCE)
    with pytest.raises(ValueError, match="not selected"):
        verify_native(package, manifest, "b" * 40)


@pytest.mark.parametrize("mutation", [None, "clients", "native-release", "qualification"])
def test_dispatch_continuation_never_replaces_the_native_identity(monkeypatch, mutation):
    import json

    from capsem_builder.release.tools import release_binaries

    controller = "b" * 40
    record = run_record()
    if mutation == "qualification":
        record["jobs"][0]["conclusion"] = "failure"
    calls = []

    def capture(_runner, *argv):
        if argv[:3] == ("gh", "release", "view"):
            return json.dumps({"isDraft": mutation == "native-release", "isPrerelease": False,
                               "targetCommitish": SOURCE, "tagName": "v0.7.2"})
        if argv[:2] == ("git", "diff"):
            return "sdk/typescript/src/index.ts" if mutation == "clients" else ""
        if argv[:3] == ("gh", "run", "list"):
            return '[{"databaseId":123}]'
        if argv[:3] == ("gh", "run", "view"):
            return json.dumps(record)
        pytest.fail(f"unexpected mutation or command: {argv}")

    def resume(_runner, **kwargs):
        calls.append(kwargs)
        return "456"

    monkeypatch.setattr(release_binaries, "_capture", capture)
    monkeypatch.setattr(release_binaries, "_resume_release", resume)
    if mutation:
        with pytest.raises(ValueError):
            release_binaries._resume_clients(release_binaries.Runner(), "v0.7.2", SOURCE, controller)
        assert not calls
    else:
        assert release_binaries._resume_clients(release_binaries.Runner(), "v0.7.2", SOURCE, controller) == "456"
        assert calls == [{"ref": f"capsem-source-{controller}", "source_commit": controller,
                          "tag": "v0.7.2", "channel": "stable", "publish": True, "client_run_id": "123"}]


@pytest.mark.parametrize("path,allowed", [("Cargo.lock", True), ("sdk/typescript/pnpm-lock.yaml", False),
                                         ("sdk/python/uv.lock", False), ("mcp/typescript/pnpm-lock.yaml", False),
                                         ("integrations/inspect-ai/uv.lock", False), ("sdk/rust/Cargo.lock", False)])
def test_controller_native_lock_is_separate_from_qualified_client_locks(tmp_path, monkeypatch, path, allowed):
    import json
    import subprocess

    from capsem_builder.release.tools import client_recovery

    def git(*args):
        return subprocess.check_output(["git", "-C", str(tmp_path), "-c", "commit.gpgsign=false", *args],
                                       text=True, stderr=subprocess.DEVNULL, timeout=10).strip()

    git("init")
    git("config", "user.name", "Fixture")
    git("config", "user.email", "fixture@example.invalid")
    target = tmp_path / path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text("version = 1\n")
    git("add", ".")
    git("commit", "-qm", "qualified source")
    source = git("rev-parse", "HEAD")
    git("tag", "v0.7.2")
    target.write_text("version = 2\n")
    git("commit", "-qam", "controller dependency change")
    original_capture = client_recovery.capture
    record = run_record()
    record.update(headSha=source, headBranch=f"capsem-source-{source}")

    def capture(root, *argv):
        if argv[:3] == ("gh", "run", "view"):
            return json.dumps(record)
        if argv[:3] == ("gh", "release", "view"):
            return json.dumps({"isDraft": False, "isPrerelease": False,
                               "targetCommitish": source, "tagName": "v0.7.2"})
        return original_capture(root, *argv)

    monkeypatch.setattr(client_recovery, "capture", capture)
    if allowed:
        assert client_recovery.original_run(tmp_path, "123", "v0.7.2") == source
    else:
        with pytest.raises(ValueError, match="client source or dependency locks"):
            client_recovery.original_run(tmp_path, "123", "v0.7.2")


def test_native_verification_uses_the_supported_public_release_reader(tmp_path, monkeypatch):
    import io
    import json
    import urllib.error
    import urllib.request
    from email.message import Message

    from capsem_builder.release.tools import client_recovery, verify_channel_downloads
    from capsem_builder.release.tools.verify_immutable_publication import file_identity

    package = tmp_path / "candidate.deb"
    package.write_bytes(b"original qualified package")
    size, digest = file_identity(package)
    manifest = {"channel": "stable", "packages": [{"name": package.name, "source_commit": SOURCE,
                "platform": "linux", "bytes": size, "digest": {"sha256": digest}}]}
    requests = []

    def open_request(request, *, timeout):
        headers = getattr(request, "headers", {})
        if headers.get("User-agent") != verify_channel_downloads.AGENT["User-Agent"]:
            raise urllib.error.HTTPError(str(request), 403, "default agent rejected", Message(), None)
        assert request.full_url == "https://release.capsem.org/assets/stable/manifest.json"
        assert timeout == verify_channel_downloads.TIMEOUT
        requests.append(request)
        return io.BytesIO(json.dumps(manifest).encode())

    monkeypatch.setattr(urllib.request, "urlopen", open_request)
    monkeypatch.setattr(client_recovery, "original_run", lambda *_: SOURCE)
    monkeypatch.setattr("sys.argv", ["client-recovery", "--run-id", "123", "--release-tag", "v0.7.2",
                                    "--native-dir", str(tmp_path)])
    client_recovery.main()
    assert len(requests) == 1
