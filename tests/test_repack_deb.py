"""Tests for scripts/repack-deb.sh.

The script is the seam between Tauri's bundler and the host install: if it
silently drops a companion binary or accepts a malformed input path, every
subsequent `dpkg -i` either installs a broken layout or explodes in dpkg
with a cryptic error. None of that is caught by pytest elsewhere because
the whole test-install stage runs inside Docker after a 30-second Tauri
build.

These tests seed a minimal fixture .deb with `dpkg-deb -b` and invoke the
script directly, so the full repack round-trip runs in under a second.
Skipped cleanly on any machine without `dpkg-deb` on PATH (macOS default);
executed in Linux CI and inside the capsem-install-test container.
"""

import contextlib
import functools
import http.server
import json
import shutil
import subprocess
import threading
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).parent.parent
SCRIPT = REPO_ROOT / "scripts" / "repack-deb.sh"
POSTINST = REPO_ROOT / "scripts" / "deb-postinst.sh"
PREINST = REPO_ROOT / "scripts" / "deb-preinst.sh"

REQUIRED_BINARIES = [
    "capsem",
    "capsem-service",
    "capsem-process",
    "capsem-tui",
    "capsem-mcp",
    "capsem-mcp-aggregator",
    "capsem-mcp-builtin",
    "capsem-gateway",
    "capsem-tray",
    "capsem-admin",
]

pytestmark = pytest.mark.skipif(
    shutil.which("dpkg-deb") is None,
    reason="dpkg-deb not on PATH (install on macOS via `brew install dpkg`)",
)


def _build_fixture_deb(workdir: Path, name: str = "capsem-fixture", version: str = "0.0.1") -> Path:
    """Build a minimal valid .deb in workdir. Returns the path to the .deb."""
    root = workdir / f"{name}-src"
    (root / "DEBIAN").mkdir(parents=True)
    (root / "DEBIAN" / "control").write_text(
        f"Package: {name}\n"
        f"Version: {version}\n"
        f"Architecture: all\n"
        f"Maintainer: Test <test@example.com>\n"
        f"Description: fixture package for repack-deb tests\n"
    )
    # Add a token file so the archive isn't empty.
    (root / "usr" / "share" / name).mkdir(parents=True)
    (root / "usr" / "share" / name / "marker.txt").write_text("fixture")
    deb_path = workdir / f"{name}_{version}_all.deb"
    subprocess.run(
        ["dpkg-deb", "-b", str(root), str(deb_path)],
        check=True,
        capture_output=True,
    )
    return deb_path


def _seed_binaries(bin_dir: Path, which: list[str] = None):
    """Drop fake executable files named like the companion binaries."""
    if which is None:
        which = REQUIRED_BINARIES
    bin_dir.mkdir(parents=True, exist_ok=True)
    for name in which:
        path = bin_dir / name
        # Minimal shell script so the file is non-empty; repack-deb copies
        # bytes, doesn't care about ELF validity.
        path.write_text(f"#!/bin/sh\necho {name} stub\n")
        path.chmod(0o755)


def _seed_config(config_dir: Path):
    """Drop a minimal materialized profile catalog."""
    profiles = config_dir / "profiles"
    (profiles / "code").mkdir(parents=True, exist_ok=True)
    (profiles / "code" / "profile.toml").write_text('id = "code"\n')
    (profiles / "code" / "enforcement.toml").write_text("# enforcement\n")


def _seed_manifest_and_local_assets(manifest: Path, assets_dir: Path) -> None:
    """Drop a v2 manifest plus tiny fake VM payloads for both supported arches."""
    digest = "a" * 64
    manifest.write_text(
        json.dumps(
            {
                "format": 2,
                "version": "9.9.9-test",
                "assets": {
                    "current": "test-release",
                    "releases": {
                        "test-release": {
                            "arches": {
                                "arm64": {"rootfs.erofs": {"hash": digest}},
                                "x86_64": {"rootfs.erofs": {"hash": digest}},
                            }
                        }
                    },
                },
                "binaries": {},
            },
            sort_keys=True,
        )
        + "\n"
    )
    for arch in ("arm64", "x86_64"):
        arch_dir = assets_dir / arch
        arch_dir.mkdir(parents=True, exist_ok=True)
        (arch_dir / f"rootfs-{digest[:16]}.erofs").write_bytes(b"fake-rootfs")


def _run_repack(
    input_deb: Path,
    bin_dir: Path,
    config_dir: Path,
    output_deb: Path = None,
    timeout: int = 30,
) -> subprocess.CompletedProcess:
    manifest = input_deb.parent / "manifest.json"
    assets_dir = input_deb.parent / "assets"
    if not manifest.exists():
        _seed_manifest_and_local_assets(manifest, assets_dir)
    args = [
        str(SCRIPT),
        "--manifest",
        manifest.resolve().as_uri(),
        str(input_deb),
        str(bin_dir),
        str(config_dir),
        str(assets_dir),
    ]
    if output_deb is not None:
        args.append(str(output_deb))
    return subprocess.run(args, capture_output=True, text=True, timeout=timeout)


def _deb_contents(deb: Path, dest: Path) -> Path:
    """Extract a .deb to dest/ and return dest."""
    subprocess.run(
        ["dpkg-deb", "-R", str(deb), str(dest)],
        check=True,
        capture_output=True,
    )
    return dest


class _QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, format: str, *args: object) -> None:
        return


class _ReleaseUserAgentRequiredHandler(_QuietHandler):
    def do_GET(self) -> None:
        user_agent = self.headers.get("User-Agent", "")
        self.server.seen_user_agents.append(user_agent)  # type: ignore[attr-defined]
        if user_agent != "CapsemReleaseValidator/1.0":
            self.send_response(403)
            self.end_headers()
            self.wfile.write(b"release validator user-agent required")
            return
        super().do_GET()


@contextlib.contextmanager
def _serve_directory(root: Path):
    handler = functools.partial(_QuietHandler, directory=str(root))
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_address[1]}"
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


@contextlib.contextmanager
def _serve_directory_requiring_release_user_agent(root: Path):
    handler = functools.partial(_ReleaseUserAgentRequiredHandler, directory=str(root))
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    server.seen_user_agents = []  # type: ignore[attr-defined]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield (
            f"http://127.0.0.1:{server.server_address[1]}",
            server.seen_user_agents,  # type: ignore[attr-defined]
        )
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def test_happy_path_adds_every_companion_binary(tmp_path):
    """All host companion binaries land in /usr/bin with mode 755."""
    fixture = _build_fixture_deb(tmp_path)
    bin_dir = tmp_path / "bin"
    config_dir = tmp_path / "target-config"
    _seed_binaries(bin_dir)
    _seed_config(config_dir)
    output = tmp_path / "out.deb"

    res = _run_repack(fixture, bin_dir, config_dir, output)
    assert res.returncode == 0, f"repack-deb.sh failed: stdout={res.stdout!r} stderr={res.stderr!r}"
    assert output.exists(), "output .deb was not created"

    extracted = _deb_contents(output, tmp_path / "extracted")
    for name in REQUIRED_BINARIES:
        binary = extracted / "usr" / "bin" / name
        assert binary.exists(), f"{name} missing from repacked .deb"
        assert binary.stat().st_mode & 0o777 == 0o755, (
            f"{name} installed with mode {oct(binary.stat().st_mode & 0o777)}, expected 0o755"
        )
    assert (extracted / "usr" / "share" / "capsem" / "profiles" / "code" / "profile.toml").exists()


def test_postinst_script_is_included(tmp_path):
    """DEBIAN/postinst is copied from scripts/deb-postinst.sh and is executable."""
    fixture = _build_fixture_deb(tmp_path)
    bin_dir = tmp_path / "bin"
    config_dir = tmp_path / "target-config"
    _seed_binaries(bin_dir)
    _seed_config(config_dir)
    output = tmp_path / "out.deb"

    res = _run_repack(fixture, bin_dir, config_dir, output)
    assert res.returncode == 0

    extracted = _deb_contents(output, tmp_path / "extracted")
    postinst = extracted / "DEBIAN" / "postinst"
    assert postinst.exists(), "DEBIAN/postinst not written"
    assert postinst.stat().st_mode & 0o777 == 0o755, (
        f"postinst mode {oct(postinst.stat().st_mode & 0o777)}, expected 0o755"
    )
    # Match on a fragment of the on-disk source so this catches a wrong-file
    # copy, not a text-munging bug.
    expected_head = POSTINST.read_text().splitlines()[0]
    assert postinst.read_text().startswith(expected_head), (
        "postinst doesn't look like scripts/deb-postinst.sh"
    )
    assert "Tester action: copy the output of this command into the bug report:" in (
        postinst.read_text()
    )


def test_preinst_script_is_included(tmp_path):
    """DEBIAN/preinst stops stale helpers before dpkg replaces binaries."""
    fixture = _build_fixture_deb(tmp_path)
    bin_dir = tmp_path / "bin"
    config_dir = tmp_path / "target-config"
    _seed_binaries(bin_dir)
    _seed_config(config_dir)
    output = tmp_path / "out.deb"

    res = _run_repack(fixture, bin_dir, config_dir, output)
    assert res.returncode == 0

    extracted = _deb_contents(output, tmp_path / "extracted")
    preinst = extracted / "DEBIAN" / "preinst"
    assert preinst.exists(), "DEBIAN/preinst not written"
    assert preinst.stat().st_mode & 0o777 == 0o755, (
        f"preinst mode {oct(preinst.stat().st_mode & 0o777)}, expected 0o755"
    )
    expected_head = PREINST.read_text().splitlines()[0]
    assert preinst.read_text().startswith(expected_head), (
        "preinst doesn't look like scripts/deb-preinst.sh"
    )
    assert "Tester action: copy the output of this command into the bug report:" in (
        preinst.read_text()
    )


def test_missing_companion_binary_fails_loudly(tmp_path):
    """Any missing binary must fail with a clear error naming the file.

    Silent drops would ship an incomplete install.
    """
    fixture = _build_fixture_deb(tmp_path)
    bin_dir = tmp_path / "bin"
    config_dir = tmp_path / "target-config"
    # Omit capsem-tray on purpose.
    _seed_binaries(bin_dir, which=[b for b in REQUIRED_BINARIES if b != "capsem-tray"])
    _seed_config(config_dir)

    res = _run_repack(fixture, bin_dir, config_dir)
    assert res.returncode != 0, (
        "repack should have failed with capsem-tray missing; "
        f"stdout={res.stdout!r} stderr={res.stderr!r}"
    )
    combined = res.stdout + res.stderr
    assert "capsem-tray" in combined, f"error message should mention capsem-tray, got: {combined!r}"


def test_path_with_embedded_newline_fails(tmp_path):
    """Newline-joined paths must fail fast, not corrupt an output .deb.

    Regression for the `just test-install` bug where a persistent
    /cargo-target volume left a stale .deb next to the current one and
    `DEB=$(ls *.deb)` captured both joined by a newline. repack-deb then
    saw one arg containing a literal newline and dpkg-deb bailed -- the
    test pins that behaviour so a future "helpful" fix doesn't silently
    consume the first path and pretend everything's fine.
    """
    fixture = _build_fixture_deb(tmp_path)
    bin_dir = tmp_path / "bin"
    config_dir = tmp_path / "target-config"
    _seed_binaries(bin_dir)
    _seed_config(config_dir)

    mangled = f"{fixture}\n{fixture}"
    res = subprocess.run(
        [str(SCRIPT), mangled, str(bin_dir), str(config_dir)],
        capture_output=True,
        text=True,
        timeout=30,
    )
    assert res.returncode != 0, (
        "repack should have failed on a newline-containing input path; "
        f"stdout={res.stdout!r} stderr={res.stderr!r}"
    )


def test_version_is_preserved_for_downgrade_and_same_version_reinstall(tmp_path):
    """DEBIAN/control's Version field is not inflated to trick the package manager."""
    fixture = _build_fixture_deb(tmp_path, version="0.0.1")
    bin_dir = tmp_path / "bin"
    config_dir = tmp_path / "target-config"
    _seed_binaries(bin_dir)
    _seed_config(config_dir)
    output = tmp_path / "out.deb"

    res = _run_repack(fixture, bin_dir, config_dir, output)
    assert res.returncode == 0

    extracted = _deb_contents(output, tmp_path / "extracted")
    control = (extracted / "DEBIAN" / "control").read_text()
    version_line = next(
        (line for line in control.splitlines() if line.startswith("Version:")),
        None,
    )
    assert version_line is not None, f"no Version: line in control: {control!r}"
    assert version_line == "Version: 0.0.1"


def test_repacked_deb_declares_tray_runtime_dependency(tmp_path):
    """The package must install libxdo3 before capsem-tray can execute."""
    fixture = _build_fixture_deb(tmp_path)
    bin_dir = tmp_path / "bin"
    config_dir = tmp_path / "target-config"
    _seed_binaries(bin_dir)
    _seed_config(config_dir)
    output = tmp_path / "out.deb"

    res = _run_repack(fixture, bin_dir, config_dir, output)
    assert res.returncode == 0, f"repack-deb.sh failed: stdout={res.stdout!r} stderr={res.stderr!r}"

    extracted = _deb_contents(output, tmp_path / "extracted")
    control = (extracted / "DEBIAN" / "control").read_text()
    depends = " ".join(
        line.strip()
        for line in control.splitlines()
        if line.startswith("Depends:") or line.startswith(" ")
    )
    assert "libxdo3" in depends


def test_explicit_manifest_url_is_packaged_without_manifest_payload(tmp_path):
    """Packages record the selected manifest URL but do not freeze a manifest copy."""
    fixture = _build_fixture_deb(tmp_path)
    bin_dir = tmp_path / "bin"
    config_dir = tmp_path / "target-config"
    manifest = tmp_path / "corp-manifest.json"
    _seed_binaries(bin_dir)
    _seed_config(config_dir)
    manifest.write_text(
        json.dumps(
            {
                "format": 2,
                "version": "9.9.9-test",
                "assets": {"current": "corp", "releases": {"corp": {"arches": {}}}},
                "binaries": {"current": "test"},
            },
            sort_keys=True,
        )
        + "\n"
    )
    output = tmp_path / "out.deb"

    res = subprocess.run(
        [
            str(SCRIPT),
            "--manifest",
            manifest.resolve().as_uri(),
            str(fixture),
            str(bin_dir),
            str(config_dir),
            "",
            str(output),
        ],
        capture_output=True,
        text=True,
        timeout=30,
    )
    assert res.returncode == 0, f"repack-deb.sh failed: stdout={res.stdout!r} stderr={res.stderr!r}"

    extracted = _deb_contents(output, tmp_path / "extracted")
    assets_dir = extracted / "usr" / "share" / "capsem" / "assets"
    assert not (assets_dir / "manifest.json").exists()
    assert (assets_dir / "manifest-origin.json").is_file()
    origin = json.loads((assets_dir / "manifest-origin.json").read_text())
    assert origin["schema"] == "capsem.manifest_origin.v1"
    assert origin["origin"] == "package"
    assert origin["source"] == manifest.resolve().as_uri()
    assert "fetched_at" in origin
    assert "packaged_at" in origin
    assert origin["package_version"] == "0.0.1"
    assert "snapshot_sha256" not in origin
    assert sorted(path.name for path in assets_dir.iterdir()) == ["manifest-origin.json"]


def test_explicit_remote_manifest_url_is_packaged_with_origin_provenance(tmp_path):
    """Remote corp/release manifest URLs are recorded without package-time fetching."""
    fixture = _build_fixture_deb(tmp_path)
    bin_dir = tmp_path / "bin"
    config_dir = tmp_path / "target-config"
    manifest_root = tmp_path / "remote"
    manifest = manifest_root / "corp-manifest.json"
    _seed_binaries(bin_dir)
    _seed_config(config_dir)
    manifest_root.mkdir()
    manifest.write_text(
        json.dumps(
            {
                "format": 2,
                "version": "remote-test",
                "assets": {"current": "corp", "releases": {"corp": {"arches": {}}}},
                "binaries": {"current": "remote"},
            },
            sort_keys=True,
        )
        + "\n"
    )
    output = tmp_path / "out.deb"

    with _serve_directory_requiring_release_user_agent(manifest_root) as (base_url, seen_user_agents):
        manifest_url = f"{base_url}/corp-manifest.json"
        res = subprocess.run(
            [
                str(SCRIPT),
                "--manifest",
                manifest_url,
                str(fixture),
                str(bin_dir),
                str(config_dir),
                "",
                str(output),
            ],
            capture_output=True,
            text=True,
            timeout=30,
        )
    assert res.returncode == 0, f"repack-deb.sh failed: stdout={res.stdout!r} stderr={res.stderr!r}"
    assert seen_user_agents == []

    extracted = _deb_contents(output, tmp_path / "extracted-remote")
    assets_dir = extracted / "usr" / "share" / "capsem" / "assets"
    assert sorted(path.name for path in assets_dir.iterdir()) == ["manifest-origin.json"]
    assert not (assets_dir / "manifest.json").exists()
    origin = json.loads((assets_dir / "manifest-origin.json").read_text())
    assert origin["schema"] == "capsem.manifest_origin.v1"
    assert origin["origin"] == "package"
    assert origin["source"] == manifest_url
    assert "fetched_at" in origin
    assert "packaged_at" in origin
    assert origin["package_version"] == "0.0.1"
    assert "snapshot_sha256" not in origin


def test_release_graph_manifest_url_is_recorded_without_conversion(tmp_path):
    """The package does not convert release graph manifests; install fetches the live URL."""
    fixture = _build_fixture_deb(tmp_path, version="1.5.0")
    bin_dir = tmp_path / "bin"
    config_dir = tmp_path / "target-config"
    graph_root = tmp_path / "remote"
    graph = graph_root / "manifest.json"
    _seed_binaries(bin_dir)
    _seed_config(config_dir)
    graph_root.mkdir()
    graph.write_text(
        json.dumps(
            {
                "schema": "capsem.release_graph.v1",
                "version": "1.5.0+assets.2030.0101.1",
                "status": "current",
                "packages": [],
                "profiles": {},
            },
            sort_keys=True,
        )
        + "\n"
    )
    output = tmp_path / "out.deb"

    with _serve_directory_requiring_release_user_agent(graph_root) as (
        base_url,
        seen_user_agents,
    ):
        manifest_url = f"{base_url}/manifest.json"
        res = subprocess.run(
            [
                str(SCRIPT),
                "--manifest",
                manifest_url,
                str(fixture),
                str(bin_dir),
                str(config_dir),
                "",
                str(output),
            ],
            capture_output=True,
            text=True,
            timeout=30,
        )

    assert res.returncode == 0, f"repack-deb.sh failed: stdout={res.stdout!r} stderr={res.stderr!r}"
    assert seen_user_agents == []

    extracted = _deb_contents(output, tmp_path / "extracted-graph")
    assets_dir = extracted / "usr" / "share" / "capsem" / "assets"
    assert sorted(path.name for path in assets_dir.iterdir()) == ["manifest-origin.json"]
    assert not (assets_dir / "manifest.json").exists()
    origin = json.loads((assets_dir / "manifest-origin.json").read_text())
    assert origin["source"] == manifest_url
    assert "snapshot_sha256" not in origin


def test_repacked_deb_payload_is_closed_and_manifest_only_for_assets(tmp_path):
    """The .deb carries binaries, profiles, and manifest metadata; VM assets stay external."""
    fixture = _build_fixture_deb(tmp_path)
    bin_dir = tmp_path / "bin"
    config_dir = tmp_path / "target-config"
    assets_dir = tmp_path / "assets"
    manifest = tmp_path / "manifest.json"
    _seed_binaries(bin_dir)
    _seed_config(config_dir)
    _seed_manifest_and_local_assets(manifest, assets_dir)
    output = tmp_path / "out.deb"

    res = subprocess.run(
        [
            str(SCRIPT),
            "--manifest",
            manifest.resolve().as_uri(),
            str(fixture),
            str(bin_dir),
            str(config_dir),
            str(assets_dir),
            str(output),
        ],
        capture_output=True,
        text=True,
        timeout=30,
    )
    assert res.returncode == 0, f"repack-deb.sh failed: stdout={res.stdout!r} stderr={res.stderr!r}"

    extracted = _deb_contents(output, tmp_path / "extracted")
    assets_dir = extracted / "usr" / "share" / "capsem" / "assets"
    assert sorted(path.name for path in assets_dir.iterdir()) == ["manifest-origin.json"]

    unexpected = []
    for path in extracted.rglob("*"):
        rel = path.relative_to(extracted).as_posix()
        if path.is_dir():
            continue
        if rel.startswith("DEBIAN/"):
            continue
        if rel.startswith("usr/bin/") and rel.removeprefix("usr/bin/") in REQUIRED_BINARIES:
            continue
        if rel == "usr/share/capsem/assets/manifest-origin.json":
            continue
        if rel.startswith("usr/share/capsem/profiles/"):
            continue
        if rel == "usr/share/capsem-fixture/marker.txt":
            continue
        unexpected.append(rel)

    assert unexpected == []


def test_repack_deb_rejects_bare_manifest_path(tmp_path):
    fixture = _build_fixture_deb(tmp_path)
    bin_dir = tmp_path / "bin"
    config_dir = tmp_path / "target-config"
    assets_dir = tmp_path / "assets"
    manifest = tmp_path / "manifest.json"
    _seed_binaries(bin_dir)
    _seed_config(config_dir)
    _seed_manifest_and_local_assets(manifest, assets_dir)

    res = subprocess.run(
        [
            str(SCRIPT),
            "--manifest",
            str(manifest),
            str(fixture),
            str(bin_dir),
            str(config_dir),
            str(assets_dir),
            str(tmp_path / "out.deb"),
        ],
        capture_output=True,
        text=True,
        timeout=30,
    )

    assert res.returncode != 0
    assert "manifest must be a URL" in (res.stdout + res.stderr)


def test_output_defaults_to_overwriting_input(tmp_path):
    """Omitting the output argument overwrites the input .deb in place."""
    fixture = _build_fixture_deb(tmp_path)
    bin_dir = tmp_path / "bin"
    config_dir = tmp_path / "target-config"
    _seed_binaries(bin_dir)
    _seed_config(config_dir)
    original_size = fixture.stat().st_size

    res = _run_repack(fixture, bin_dir, config_dir)  # no output arg
    assert res.returncode == 0

    # Original .deb path still exists and is now larger (companion binaries added).
    assert fixture.exists()
    assert fixture.stat().st_size > original_size, (
        "in-place repack should produce a larger .deb after adding binaries"
    )
