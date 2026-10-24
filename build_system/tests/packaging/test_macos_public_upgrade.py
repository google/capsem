"""Public metadata and labelled synthetic package payloads; no installation."""

from __future__ import annotations

import json
from pathlib import Path
from types import SimpleNamespace

import pytest
from capsem_builder import crypto
from capsem_builder.release.tools import release_macos_pairing

from build_system.tests.packaging.test_macos_binary_apply import inputs as inputs

PUBLIC = (
    Path(__file__).resolve().parents[3] / "tests/fixtures/release/public-stable-066-manifest.json"
)


def test_actual_public_metadata_has_distinct_legacy_content_domain():
    assert (
        crypto.digest_file(PUBLIC)
        == "2b04fb53657ee3146c66054c7bd4177248eec6ba21bb0e51f33482ebb4b966c8"
    )
    document = json.loads(PUBLIC.read_text())
    identity = release_macos_pairing.legacy_profile_identity(document, architecture="arm64")
    assert identity["domain"] == "legacy_profiles"
    assert set(identity["profiles"]) == {"code", "co-work"}
    assert "runtime_sha256" not in identity


@pytest.mark.parametrize("damage", ["runtime", "empty", "revoked", "arch", "digest"])
def test_legacy_identity_refuses_ambiguous_or_incomplete_original_metadata(damage):
    document = json.loads(PUBLIC.read_text())
    if damage == "runtime":
        document["runtime"] = {"revision": "invented"}
    elif damage == "empty":
        document["profiles"] = {}
    elif damage == "revoked":
        document["profiles"]["code"]["status"] = "revoked"
    elif damage == "arch":
        document["profiles"]["code"]["architectures"] = []
    else:
        document["profiles"]["code"]["architectures"][0]["images"][0]["digest"] = {}
    with pytest.raises(ValueError):
        release_macos_pairing.legacy_profile_identity(document, architecture="arm64")


def public_pair(inputs):
    """Synthetic archive mechanics retain real original profile metadata only."""
    before, after = inputs
    document = json.loads(before[1].read_text())
    document.pop("runtime")
    document["profiles"] = json.loads(PUBLIC.read_text())["profiles"]
    before[1].write_text(json.dumps(document))
    return before, after


def test_public_pair_tags_both_domains_and_keeps_binary_only_strict(inputs):
    before, after = public_pair(inputs)
    pair = release_macos_pairing.validate_public_pair(before=before, after=after, channel="nightly")
    assert pair["scope"] == "public_predecessor_upgrade"
    assert pair["before"]["content_identity"]["domain"] == "legacy_profiles"
    assert pair["after"]["content_identity"]["domain"] == "runtime"
    assert "runtime_sha256" not in pair["before"]
    with pytest.raises((ValueError, RuntimeError)):
        release_macos_pairing.validate_pair(before=before, after=after, channel="nightly")


@pytest.mark.parametrize(
    "damage", ["same", "after_profiles", "after_empty", "old_version", "payload", "leading_zero"]
)
def test_public_pair_does_not_relax_package_or_version_authority(inputs, damage):
    before, after = public_pair(inputs)
    if damage == "same":
        after = before
    else:
        path = after[1]
        doc = json.loads(path.read_text())
        if damage == "after_profiles":
            doc["profiles"] = json.loads(PUBLIC.read_text())["profiles"]
        elif damage == "after_empty":
            doc.pop("runtime")
        elif damage == "old_version":
            doc["packages"][0]["version"] = "0.6.0"
        elif damage == "leading_zero":
            doc["packages"][0]["version"] = "01.7.1"
            for binary in doc["packages"][0]["binaries"]:
                binary["version"] = "01.7.1"
        else:
            doc["packages"][0]["binaries"][0]["digest"]["sha256"] = "f" * 64
        path.write_text(json.dumps(doc))
    with pytest.raises((ValueError, RuntimeError)):
        release_macos_pairing.validate_public_pair(before=before, after=after, channel="nightly")


def test_existing_tart_mode_stages_originals_without_requiring_fake_embedded_url(
    inputs, tmp_path, monkeypatch
):
    root = Path(__file__).resolve().parents[3]
    monkeypatch.syspath_prepend(str(root / "build_system/packaging/macos"))
    import macos_binary_apply

    before, after = public_pair(inputs)
    args = SimpleNamespace(
        before_package=before[0],
        before_manifest_file=before[1],
        before_sbom=before[2],
        package=after[0],
        manifest_file=after[1],
        sbom=after[2],
        channel="nightly",
        manifest_url="http://127.0.0.1:19000/candidate/assets/nightly/manifest.json",
        public_predecessor_upgrade=True,
    )
    share = tmp_path / "share"
    package = share / "candidate" / after[0].name
    package.parent.mkdir(parents=True)
    package.write_bytes(after[0].read_bytes())
    pair = macos_binary_apply.prepare(args, root=root, share=share, guest_package=package)
    assert pair["scope"] == "public_predecessor_upgrade"
    assert (share / "before/manifest.json").read_bytes() == before[1].read_bytes()
    assert (share / "candidate/assets/nightly/manifest.json").read_bytes() == before[1].read_bytes()
    assert pair["before"]["metadata_manifest_url"] != args.manifest_url
    args.before_sbom = None
    with pytest.raises(ValueError):
        macos_binary_apply.validate_options(args)


def test_scope_uses_one_existing_apply_and_explicit_original_feed_selection():
    source = (
        PUBLIC.parents[3] / "build_system/packaging/macos/macos-tart-binary-apply.sh"
    ).read_text()
    assert source.count("capsem_pair_http request") == 1
    assert source.index('"$CAPSEM" update --assets --manifest "$MANIFEST_URL"') < source.index(
        'promote "$SHARE/original-manifest.json"'
    )
    assert source.index("capsem_public_state capture") < source.index(
        'promote "$SHARE/original-manifest.json"'
    )
    tail = source.split("capsem_pair_http request", 1)[1]
    assert "launchctl kickstart" not in tail
    assert "capsem_public_state preserved" in tail
    assert 'capsem_wait_for_assets "$SHARE/public-upgrade-assets-readiness.json"' in tail


@pytest.mark.parametrize("scope,phase,legacy", [
    ("binary_apply_only", "before", False),
    ("public_predecessor_upgrade", "before", True),
    ("public_predecessor_upgrade", "after", False),
])
def test_actual_bash_pair_verification_preserves_optional_argv(tmp_path, scope, phase, legacy):
    import shlex
    import subprocess

    source = Path(__file__).resolve().parents[3] / "build_system/packaging/macos/macos-tart-binary-apply.sh"
    capture = tmp_path / "verify-argv"
    script = f'''
set -euo pipefail
source {shlex.quote(str(source))}
SHARE={shlex.quote(str(tmp_path))}
CAPSEM=synthetic_capsem
VERIFY=synthetic-verifier.py
MANIFEST_URL=http://source-fixture.invalid/manifest.json
CHANNEL=stable
capsem_pair_field() {{
    case "$*" in
        *'--field scope') echo {shlex.quote(scope)} ;;
        *'--field version') echo 0.7.1 ;;
        *'--field package') echo /source-fixture.pkg ;;
        *'--field metadata_manifest_url') echo http://source-fixture.invalid/manifest.json ;;
        *) return 1 ;;
    esac
}}
python3() {{
    if [ "$1" = -m ]; then echo capsem; else printf '%s\\n' "$@" > {shlex.quote(str(capture))}; fi
}}
verify_binary_cohort() {{ return 0; }}
synthetic_capsem() {{ echo 0.7.1; }}
function /usr/sbin/pkgutil {{ echo 'version: 0.7.1'; }}
function /usr/libexec/PlistBuddy {{ echo 0.7.1; }}
capsem_pair_verify {shlex.quote(phase)}
'''
    result = subprocess.run(["/bin/bash", "-c", script], capture_output=True, text=True, timeout=10)
    assert result.returncode == 0, result.stderr
    argv = capture.read_text().splitlines()
    assert "" not in argv
    assert ("--legacy-profile-predecessor" in argv) is legacy
    assert argv[0] == "synthetic-verifier.py" and "--artifact" in argv
