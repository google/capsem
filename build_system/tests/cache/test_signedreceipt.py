"""Private signed handoff source doubles; no real codesigner or product runs."""

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import pytest
from capsem_builder.cache import signedreceipt as subject

ROOT = Path(__file__).resolve().parents[3]
ENTITLEMENTS_NAME = Path("build_system/packaging/macos/entitlements.plist").name


@pytest.fixture
def runner(tmp_path):
    root = tmp_path / "root"
    directory = root / "build_system/packaging/macos"
    directory.mkdir(parents=True)
    script = directory / "run_signed.sh"
    shutil.copy2(ROOT / "build_system/packaging/macos/run_signed.sh", script)
    (directory / ENTITLEMENTS_NAME).write_text("<entitlements/>\n")
    original = root / "cache/target/cargo/release/deps/source-double"
    original.parent.mkdir(parents=True)
    original.write_text(
        f'#!{sys.executable}\nimport json,os,sys\nprint(json.dumps({{"assets":os.environ.get("CAPSEM_ASSETS_DIR"),"args":sys.argv[1:]}}))\n'
    )
    original.chmod(0o755)
    tools = tmp_path / "tools"
    tools.mkdir()
    code = (
        f"#!{sys.executable}\n"
        + """import os,sys,shutil
from pathlib import Path
name=Path(sys.argv[0]).name
args=sys.argv[1:]
if name=="uname": print("Darwin")
elif name=="perl": pass
elif name=="plutil":
 if args[-1]=="-": sys.stdin.read()
 print("<entitlements/>")
elif name=="codesign":
 if os.environ.get("SOURCE_DOUBLE_SIGNATURE_FAIL"): sys.exit(1)
 if "--display" in args: print("<entitlements/>"+" "*int(os.environ.get("SOURCE_DOUBLE_ENTITLEMENTS_PADDING","0")))
elif name=="cp": shutil.copy2(*[a for a in args if a!="-c"])
elif name=="stat":
 for path in args[2:]:
  s=os.stat(path)
  print(f"{s.st_dev}:{s.st_ino}:{s.st_mode:o}:{s.st_size}:{s.st_mtime_ns}:{s.st_ctime_ns}")
else: raise AssertionError(name)
"""
    )
    for name in ("uname", "perl", "plutil", "codesign", "cp", "stat"):
        path = tools / name
        path.write_text(code)
        path.chmod(0o755)
    environment = {**os.environ, "PATH": str(tools) + os.pathsep + os.environ["PATH"]}
    return root, script, original, environment


def test_prepare_only_returns_actual_copy_without_executing_it(runner):
    root, script, original, environment = runner
    result = subprocess.run(
        ["bash", str(script), "--prepare-only", sys.executable, "65536", str(original)],
        env=environment,
        capture_output=True,
        text=True,
        timeout=15,
    )
    assert result.returncode == 0, result.stderr
    receipt = subject.PreparedExecutable.model_validate_json(result.stdout)
    receipt.verify(
        original=original,
        policy=(script.with_name(ENTITLEMENTS_NAME), script),
        assets_dir=root / "assets",
    )
    assert receipt.executable.path != original and receipt.executable.path.is_file()
    assert receipt.argv0 == str(original)
    assert 'exec -a "$original" "$binary" "$@"' in script.read_text()


def test_normal_runner_exec_and_assets_contract_is_unchanged(runner):
    root, script, original, environment = runner
    result = subprocess.run(
        ["bash", str(script), str(original), "--source-double"],
        env=environment,
        capture_output=True,
        text=True,
        timeout=15,
    )
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout) == {
        "assets": str(root / "assets"),
        "args": ["--source-double"],
    }


def test_signing_pipeline_consumes_entitlements_before_exiting(runner):
    _, script, original, environment = runner
    # Exceed the pipe buffer so an early-exiting plutil double deterministically
    # breaks codesign's stdout, rather than depending on hosted scheduling.
    environment["SOURCE_DOUBLE_ENTITLEMENTS_PADDING"] = "131072"
    result = subprocess.run(
        ["bash", str(script), str(original), "--source-double"],
        env=environment,
        capture_output=True,
        text=True,
        timeout=15,
    )
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout)["args"] == ["--source-double"]


def test_failed_existing_signature_validation_cannot_emit_receipt(runner):
    _, script, original, environment = runner
    environment["SOURCE_DOUBLE_SIGNATURE_FAIL"] = "1"
    result = subprocess.run(
        ["bash", str(script), "--prepare-only", sys.executable, "65536", str(original)],
        env=environment,
        capture_output=True,
        text=True,
        timeout=15,
    )
    assert result.returncode != 0 and not result.stdout


@pytest.fixture
def inputs(tmp_path):
    paths = [tmp_path / name for name in ("original", "policy", "runner", "signed")]
    for path in paths:
        path.write_text("controlled source double\n")
    source = subject.Snapshot(files=tuple(subject.FileStamp.capture(path) for path in paths[:3]))
    signed = subject.Snapshot(files=(subject.FileStamp.capture(paths[3]),))
    return paths, source, signed


@pytest.mark.parametrize("changed", range(4))
def test_same_size_replacement_of_any_input_rejects_handoff(inputs, changed, capsys):
    paths, source, signed = inputs
    paths[changed].unlink()
    paths[changed].write_text("controlled source double\n")
    with pytest.raises(ValueError, match="replaced"):
        subject.main(
            [
                "emit",
                "65536",
                str(paths[0]),
                str(paths[3]),
                str(paths[0].parent / "assets"),
                source.model_dump_json(),
                signed.model_dump_json(),
            ]
        )
    assert capsys.readouterr().out == ""


def test_valid_receipt_revalidates_original_copy_policy_and_bindings(inputs, capsys):
    paths, source, signed = inputs
    assets = paths[0].parent / "assets"
    subject.main(
        [
            "emit",
            "65536",
            str(paths[0]),
            str(paths[3]),
            str(assets),
            source.model_dump_json(),
            signed.model_dump_json(),
        ]
    )
    receipt = subject.PreparedExecutable.model_validate_json(capsys.readouterr().out)
    receipt.verify(original=paths[0], policy=(paths[1], paths[2]), assets_dir=assets)
    with pytest.raises(ValueError, match="foreign"):
        receipt.verify(original=paths[0], policy=(paths[2], paths[1]), assets_dir=assets)
    paths[3].write_text("mutated signed source double")
    with pytest.raises(ValueError, match="replaced"):
        receipt.verify(original=paths[0], policy=(paths[1], paths[2]), assets_dir=assets)


@pytest.mark.parametrize("kind", ["link", "directory", "relative"])
def test_nonregular_or_relative_handoff_sources_are_refused(tmp_path, kind):
    path = tmp_path / "input"
    if kind == "link":
        path.symlink_to(__file__)
    elif kind == "directory":
        path.mkdir()
    else:
        path = Path("relative")
    with pytest.raises(ValueError):
        subject.FileStamp.capture(path)


def test_typed_receipt_and_byte_budget_fail_closed(inputs, capsys):
    paths, source, _ = inputs
    with pytest.raises(ValueError, match="budget"):
        subject.main(["capture", "1", str(paths[0])])
    assert capsys.readouterr().out == ""
    with pytest.raises(ValueError):
        subject.Snapshot.model_validate({**source.model_dump(), "foreign": True})
