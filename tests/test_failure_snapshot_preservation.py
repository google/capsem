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


def test_direct_worktree_failure_evidence_uses_the_common_cache_stage():
    from capsem_builder.cache.config import load_paths

    paths = load_paths(service.PROJECT_ROOT)
    assert paths.policy.control is not None
    expected = paths.stage(paths.policy.control.failure_artifacts.stage)
    assert expected == service.failures.ARTIFACTS_ROOT


def test_synthetic_linked_worktree_and_explicit_authority_share_evidence(tmp_path, monkeypatch):
    import shutil

    from capsem_builder.cache.config import load_policy

    primary = tmp_path / "primary"
    worktree = tmp_path / "linked"
    metadata = primary / ".git" / "worktrees" / "linked"
    metadata.mkdir(parents=True)
    (metadata / "commondir").write_text("../..")
    worktree.mkdir()
    (worktree / ".git").write_text(f"gitdir: {metadata}\n")
    (worktree / "config").mkdir()
    shutil.copy2(service.PROJECT_ROOT / "config/cache.toml", worktree / "config/cache.toml")
    policy = load_policy(worktree)
    assert policy.control is not None
    monkeypatch.delenv(policy.authority_environment, raising=False)
    expected = primary / policy.root / policy.stages[policy.control.failure_artifacts.stage].path
    assert service.failures.artifact_root(worktree) == expected
    assert not expected.exists(), "path resolution must not create or mutate evidence"
    external = tmp_path / "explicit-authority"
    monkeypatch.setenv(policy.authority_environment, str(external))
    assert service.failures.artifact_root(worktree) == external / policy.root / policy.stages[policy.control.failure_artifacts.stage].path
