"""A teardown snapshot must not overwrite a prior read-only snapshot."""

from unittest.mock import patch

from helpers import service


def test_repeated_failure_preserves_keep_distinct_complete_snapshots(
    tmp_path, monkeypatch, capsys
):
    source = tmp_path / "capsem-test-owned"
    source.mkdir()
    blob = source / "immutable-blob"
    blob.write_text("verified blob")
    blob.chmod(0o444)
    (source / "link").symlink_to("immutable-blob")
    log = source / "process.log"
    log.write_text("before teardown")
    evidence = tmp_path / "evidence"
    monkeypatch.setenv("CAPSEM_TEST_ARTIFACTS_ROOT", str(evidence))
    with patch("helpers.service.time.strftime", return_value="fixed-time"):
        service.preserve_tmp_dir_on_failure(source, force=True)
        log.write_text("after teardown")
        service.preserve_tmp_dir_on_failure(source, force=True)
    snapshots = sorted(evidence.glob("*/capsem-test-owned*"))
    assert len(snapshots) == 2
    assert {path.joinpath("process.log").read_text() for path in snapshots} == {
        "before teardown",
        "after teardown",
    }
    for path in snapshots:
        assert path.joinpath("immutable-blob").read_text() == "verified blob"
        assert path.joinpath("immutable-blob").stat().st_mode & 0o777 == 0o444
        assert path.joinpath("link").is_symlink()
        assert path.joinpath("link").readlink().as_posix() == "immutable-blob"
    report = capsys.readouterr().err
    assert report.count("errors=0") == 2, report
    assert "Permission denied" not in report and "File exists" not in report, report
