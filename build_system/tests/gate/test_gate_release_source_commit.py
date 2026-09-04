"""One immutable source identity for a release qualification."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import pytest

PROJECT_ROOT = Path(__file__).resolve().parents[3]


def _relocated_prefix(original, tmp_path: Path):
    return original.prefix.model_copy(
        update={
            "parent": str(tmp_path),
            "build_cache": str(tmp_path / "cache" / "target" / "prefix-products"),
            "cargo_target": str(tmp_path / "cache" / "target" / "cargo"),
        }
    )


def _git(root: Path, *args: str, check: bool = True) -> str:
    return subprocess.run(
        ["git", *args],
        cwd=root,
        check=check,
        capture_output=True,
        text=True,
    ).stdout.strip()


@pytest.fixture
def committed_source(tmp_path: Path) -> tuple[Path, str, str]:
    remote = tmp_path / "origin.git"
    _git(tmp_path, "init", "-q", "--bare", str(remote))
    root = tmp_path / "source"
    root.mkdir()
    _git(root, "init", "-q", "-b", "main")
    _git(root, "config", "user.email", "gate@example.com")
    _git(root, "config", "user.name", "Gate")
    _git(root, "config", "commit.gpgsign", "false")

    (root / ".gitignore").write_text("private/\ncache/target/\n", encoding="utf-8")
    (root / "tracked.txt").write_text("first\n", encoding="utf-8")
    _git(root, "add", ".gitignore", "tracked.txt")
    _git(root, "commit", "-qm", "first")
    first = _git(root, "rev-parse", "HEAD")

    (root / "tracked.txt").write_text("second\n", encoding="utf-8")
    _git(root, "commit", "-qam", "second")
    second = _git(root, "rev-parse", "HEAD")
    _git(root, "remote", "add", "origin", str(remote))
    _git(root, "push", "-q", "-u", "origin", "main")

    (root / "tracked.txt").write_text("dirty outer checkout\n", encoding="utf-8")
    (root / "untracked.txt").write_text("not release source\n", encoding="utf-8")
    (root / "private" / "tauri").mkdir(parents=True)
    (root / "private" / "tauri" / "capsem.key").write_text("secret\n", encoding="utf-8")
    return root, first, second


@pytest.mark.parametrize(
    "invalid",
    [
        "HEAD",
        "main",
        "a" * 39,
        "a" * 41,
        "A" * 40,
        "g" * 40,
        "a" * 39 + "\n",
    ],
)
def test_source_commit_is_one_canonical_full_git_identity(invalid: str) -> None:
    from capsem_builder.gate.sourcecommit import SourceCommit

    with pytest.raises(ValueError, match="40-character lowercase hexadecimal"):
        SourceCommit(invalid)

    selected = SourceCommit("0123456789abcdef" * 2 + "01234567")
    assert str(selected) == "0123456789abcdef" * 2 + "01234567"


def test_release_commit_must_already_belong_to_local_main(
    committed_source: tuple[Path, str, str],
) -> None:
    from capsem_builder.gate.sourcecommit import SourceCommit, require_local_main

    source, first, second = committed_source
    require_local_main(source, SourceCommit(first))
    require_local_main(source, SourceCommit(second))

    side = _git(source, "commit-tree", "HEAD^{tree}", "-m", "not on main")
    with pytest.raises(Exception, match="local main"):
        require_local_main(source, SourceCommit(side))


def test_checkout_source_identity_uses_the_same_typed_full_commit(
    committed_source: tuple[Path, str, str],
) -> None:
    from capsem_builder.gate.sourcecommit import SourceCommit, source_commit_for_checkout

    source, _first, second = committed_source
    assert source_commit_for_checkout(source) == SourceCommit(second)


def test_exact_commit_snapshot_ignores_newer_and_dirty_outer_source(
    committed_source: tuple[Path, str, str], tmp_path: Path
) -> None:
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate import snapshot
    from capsem_builder.gate.sourcecommit import SourceCommit

    source, first, _second = committed_source
    target = tmp_path / "prefix"
    snapshot.populate_commit(source, target, gate_config.load(PROJECT_ROOT), SourceCommit(first))

    assert _git(target, "rev-parse", "HEAD") == first
    assert _git(target, "branch", "--show-current") == "", "release source is detached"
    assert (target / "tracked.txt").read_text(encoding="utf-8") == "first\n"
    assert not (target / "untracked.txt").exists()
    assert (target / "private" / "tauri" / "capsem.key").read_text(encoding="utf-8") == "secret\n"
    assert _git(target, "remote", "get-url", "origin") == _git(
        source, "remote", "get-url", "origin"
    )
    assert _git(target, "remote", "get-url", "origin") != str(source)
    assert _git(target, "status", "--porcelain", "--untracked-files=all") == ""


def test_relative_origin_is_canonicalized_before_the_prefix_moves(
    committed_source: tuple[Path, str, str], tmp_path: Path
) -> None:
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate import snapshot
    from capsem_builder.gate.sourcecommit import SourceCommit

    source, first, _second = committed_source
    _git(source, "remote", "set-url", "origin", "../origin.git")
    target = tmp_path / "elsewhere" / "prefix"

    snapshot.populate_commit(source, target, gate_config.load(PROJECT_ROOT), SourceCommit(first))

    assert _git(target, "remote", "get-url", "origin") == str((tmp_path / "origin.git").resolve())


def test_release_prefix_name_is_the_complete_commit_not_a_truncation() -> None:
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate import prefix
    from capsem_builder.gate.sourcecommit import SourceCommit

    commit = SourceCommit("0123456789abcdef" * 2 + "01234567")
    config = gate_config.load(PROJECT_ROOT)

    assert prefix.for_source_commit(config, commit) == prefix.parent_dir(config) / str(commit)


@pytest.mark.parametrize(
    ("argv", "slot"),
    [
        (["release-binaries", "nightly"], 2),
        (["release-profile", "nightly", "code"], 3),
    ],
)
def test_release_cli_requires_the_explicit_source_commit(argv: list[str], slot: int) -> None:
    from capsem_builder.gate import cli
    from capsem_builder.gate.sourcecommit import SourceCommit

    with pytest.raises(SystemExit):
        cli.build_parser().parse_args(argv)

    commit = "0123456789abcdef" * 2 + "01234567"
    parsed = cli.build_parser().parse_args([*argv, commit])
    assert parsed.source_commit == SourceCommit(commit)
    assert [*argv, commit][slot] == str(parsed.source_commit)


def test_release_prefix_reexec_uses_commit_identity_not_source_checkout(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from capsem_builder.gate import buildcache, cachelayout, cachetooling, cargotarget, prefix
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate.sourcecommit import SourceCommit

    commit = SourceCommit("0123456789abcdef" * 2 + "01234567")
    original = gate_config.load(PROJECT_ROOT)
    config = original.model_copy(update={"prefix": _relocated_prefix(original, tmp_path)})
    populated: list[tuple[Path, SourceCommit]] = []
    environments: list[dict[str, str]] = []

    class FailedRunner:
        def note(self, _message: str) -> None:
            pass

        def run(self, _argv, *, cwd, env, check) -> int:
            assert check is False
            environments.append(env)
            return 1

    def populate(_source: Path, target: Path, _config, selected: SourceCommit) -> None:
        target.mkdir()
        populated.append((target, selected))

    monkeypatch.setattr(prefix, "sweep", lambda _config: [])
    monkeypatch.setattr(prefix.snapshot, "populate_commit", populate)
    monkeypatch.setattr(buildcache, "export", lambda *args: None)
    monkeypatch.setattr(cachetooling, "record_use", lambda *args, **kwargs: None)

    assert (
        prefix.run_from_private_copy(
            FailedRunner(), config, ["release-binaries", "nightly", str(commit)], commit=commit
        )
        == 1
    )
    assert populated == [(tmp_path / str(commit), commit)]
    assert environments == [
        {
            config.environment.source_checkout: str(config.root),
            cachelayout.cache_paths(config).policy.authority_environment: str(config.root),
            config.environment.source_commit: str(commit),
            # Named here rather than merely tolerated: the child compiles into
            # one shared build directory, and it learns that from the exported
            # environment. A release prefix that did not carry it would take a
            # cold build on every dispatch.
            config.environment.cargo_target: str(cargotarget.path(config)),
            **cachetooling.environment(
                config,
                key=str(commit),
                source_root=tmp_path / str(commit),
            ),
        }
    ]


def test_exact_commit_prefix_has_a_nonblocking_cross_process_lease(tmp_path: Path) -> None:
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate import prefix
    from capsem_builder.gate.sourcecommit import SourceCommit

    commit = SourceCommit("0123456789abcdef" * 2 + "01234567")
    original = gate_config.load(PROJECT_ROOT)
    config = original.model_copy(update={"prefix": _relocated_prefix(original, tmp_path)})
    path = prefix.for_source_commit(config, commit)

    probe = (
        "import sys\n"
        "from pathlib import Path\n"
        "from capsem_builder.gate import config as gate_config\n"
        "from capsem_builder.gate.errors import PrefixBusy\n"
        "from capsem_builder.gate.prefixlease import lease\n"
        "base = gate_config.load(Path(sys.argv[1]))\n"
        "config = base.model_copy(update={'prefix': base.prefix.model_copy("
        "update={'parent': sys.argv[2]})})\n"
        "try:\n"
        "    with lease(config, Path(sys.argv[3])):\n"
        "        pass\n"
        "except PrefixBusy:\n"
        "    raise SystemExit(23)\n"
    )
    with prefix.lease(config, path):
        child = subprocess.run(
            [sys.executable, "-c", probe, str(PROJECT_ROOT), str(tmp_path), str(path)],
            cwd=PROJECT_ROOT,
            check=False,
            capture_output=True,
            text=True,
        )

    assert child.returncode == 23, child.stderr


def test_forged_source_marker_cannot_bypass_exact_prefix_materialization(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate import prefix
    from capsem_builder.gate.sourcecommit import SourceCommit

    commit = SourceCommit("0123456789abcdef" * 2 + "01234567")
    original = gate_config.load(PROJECT_ROOT)
    config = original.model_copy(update={"prefix": _relocated_prefix(original, tmp_path)})

    monkeypatch.setenv(config.environment.source_commit, str(commit))
    assert prefix.active(config, commit) is False

    selected = prefix.for_source_commit(config, commit)
    selected.symlink_to(PROJECT_ROOT, target_is_directory=True)
    with pytest.raises(Exception, match="must not be a symlink"):
        prefix.run_from_private_copy(object(), config, [], commit=commit)


def test_ty_refuses_a_raw_string_at_source_commit_seams() -> None:
    fixture = (
        PROJECT_ROOT / "build_system/tests/gate/fixtures/typecheck/gate_vocabulary_strings.py.txt"
    )
    content = fixture.read_text(encoding="utf-8")

    assert "SourceCommit" in content
    assert 'require_local_main(Path.cwd(), "0" * 40)' in content


def test_source_transport_ref_is_create_or_verify_not_mutable(
    committed_source: tuple[Path, str, str], tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate import snapshot
    from capsem_builder.gate.sourcecommit import SourceCommit
    from capsem_builder.release.tools import publish_release_source as module

    source, first, second = committed_source
    target = tmp_path / "qualified"
    snapshot.populate_commit(source, target, gate_config.load(PROJECT_ROOT), SourceCommit(first))
    monkeypatch.setattr(module, "ROOT", target)

    short = module.publish(first, "capsem-source-{source_commit}")
    assert short == f"capsem-source-{first}"
    assert module.publish(first, "capsem-source-{source_commit}") == short
    assert _git(source, "ls-remote", "--refs", "origin", f"refs/tags/{short}").split()[0] == first

    _git(source, "push", "-q", "--force", "origin", f"{second}:refs/tags/{short}")
    with pytest.raises(RuntimeError, match="points at"):
        module.publish(first, "capsem-source-{source_commit}")


def test_same_commit_source_ref_creation_race_converges(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from capsem_builder.release.tools import publish_release_source as module

    commit = "1" * 40
    observed = iter((None, commit, commit))

    class RejectedPush:
        returncode = 1
        stderr = "another publisher won"

    monkeypatch.setattr(module, "_require_source", lambda _commit: None)
    monkeypatch.setattr(module, "_require_remote_main", lambda _commit: None)
    monkeypatch.setattr(module, "_remote_ref", lambda _ref: next(observed))
    monkeypatch.setattr(module, "_run", lambda *_args, **_kwargs: RejectedPush())

    assert module.publish(commit, "capsem-source-{source_commit}") == f"capsem-source-{commit}"


@pytest.mark.parametrize("workflow_name", ["release.yaml", "release-assets.yaml"])
def test_release_workflow_evidence_pins_every_checkout_to_required_source_commit(
    workflow_name: str,
) -> None:
    workflow = (PROJECT_ROOT / ".github" / "workflows" / workflow_name).read_text(encoding="utf-8")
    trigger = workflow.split("\nconcurrency:", maxsplit=1)[0]
    source_input = trigger.split("source_commit:", maxsplit=1)[1]
    assert "required: true" in "\n".join(source_input.splitlines()[:5])
    assert "SOURCE_COMMIT: ${{ inputs.source_commit }}" in workflow

    checkout_blocks = workflow.split("uses: actions/checkout@")[1:]
    assert checkout_blocks
    for block in checkout_blocks:
        step = block.split("\n      - ", maxsplit=1)[0]
        assert "ref: ${{ inputs.source_commit }}" in step

    assert 'test "$GITHUB_SHA" = "$SOURCE_COMMIT"' in workflow
    assert 'test "$GITHUB_REF" = "refs/tags/capsem-source-$SOURCE_COMMIT"' in workflow
    assert 'test "$(git rev-parse HEAD)" = "$SOURCE_COMMIT"' in workflow


def test_reusable_release_workflows_receive_the_same_source_commit() -> None:
    binary = (PROJECT_ROOT / ".github/workflows/release.yaml").read_text(encoding="utf-8")
    profile = (PROJECT_ROOT / ".github/workflows/release-assets.yaml").read_text(encoding="utf-8")
    for workflow in (binary, profile):
        assert "uses: ./.github/workflows/release-runtime-preflight.yaml" in workflow
        assert "uses: ./.github/workflows/fast-gate.yaml" in workflow
        assert workflow.count("source_commit: ${{ inputs.source_commit }}") >= 3

    for workflow_name in ("release-runtime-preflight.yaml", "fast-gate.yaml"):
        workflow = (PROJECT_ROOT / ".github" / "workflows" / workflow_name).read_text(
            encoding="utf-8"
        )
        assert "source_commit:" in workflow
        assert "ref: ${{ inputs.source_commit" in workflow

    channel = (PROJECT_ROOT / ".github/workflows/release-channel.yaml").read_text(encoding="utf-8")
    assert "source_commit:" in channel
    assert (
        "ref: ${{ inputs.artifact_run_id != '' && github.sha || "
        "inputs.source_commit || github.sha }}"
    ) in channel
