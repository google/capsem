from __future__ import annotations

import importlib.util
import json
from pathlib import Path
from urllib.request import Request

import pytest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "resolve-reusable-profile-assets.py"
SPEC = importlib.util.spec_from_file_location("resolve_reusable_profile_assets", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
RESOLVE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RESOLVE)


def _selection(
    *,
    channel: str = "nightly",
    profile: str = "code",
    revision: str = "2026.06.08.7",
) -> dict[str, object]:
    return {
        "schema": "capsem.admin.release_validate.v1",
        "ok": True,
        "channel": channel,
        "profile": profile,
        "profile_revision": revision,
        "publication_identity": f"profile-{channel}-{profile}-{revision}",
        "profile_path": f"config/profiles/{profile}/profile.toml",
    }


def _artifacts(run_id: int, *, complete: bool = True) -> list[dict[str, object]]:
    names = [
        "profile-release-selection",
        "vm-assets-arm64",
        "vm-assets-x86_64",
    ]
    if not complete:
        names.remove("vm-assets-arm64")
    return [
        {
            "id": run_id * 10 + index,
            "name": name,
            "expired": False,
            "archive_download_url": (f"https://api.github.com/artifacts/{run_id}/{name}"),
        }
        for index, name in enumerate(names)
    ]


def test_newest_exact_completed_run_reuses_one_complete_artifact_cohort() -> None:
    expected = _selection()
    runs = [
        {"id": 50, "status": "in_progress"},
        {"id": 49, "status": "completed"},
        {"id": 48, "status": "completed"},
        {"id": 47, "status": "completed"},
    ]
    artifacts = {
        49: _artifacts(49, complete=False),
        48: _artifacts(48),
        47: _artifacts(47),
    }
    selections = {
        48: _selection(),
        47: _selection(revision="older"),
    }

    selected = RESOLVE.select_reusable_run(
        runs=runs,
        current_run_id=50,
        expected_selection=expected,
        artifact_loader=lambda run_id: artifacts[run_id],
        selection_loader=lambda artifact: selections[int(artifact["id"]) // 10],
    )

    assert selected == 48


def test_reuse_never_mixes_runs_or_accepts_expired_duplicate_or_wrong_selection() -> None:
    expected = _selection()
    runs = [
        {"id": 60, "status": "completed"},
        {"id": 59, "status": "completed"},
        {"id": 58, "status": "completed"},
        {"id": 57, "status": "completed"},
    ]
    expired = _artifacts(60)
    expired[1]["expired"] = True
    duplicate = _artifacts(59)
    duplicate.append(dict(duplicate[-1]))
    artifacts = {
        60: expired,
        59: duplicate,
        58: _artifacts(58, complete=False),
        57: _artifacts(57),
    }

    selected = RESOLVE.select_reusable_run(
        runs=runs,
        current_run_id=61,
        expected_selection=expected,
        artifact_loader=lambda run_id: artifacts[run_id],
        selection_loader=lambda _artifact: _selection(profile="co-work"),
    )

    assert selected is None


def test_artifact_cohort_rejects_non_github_download_origin() -> None:
    artifacts = _artifacts(70)
    artifacts[0]["archive_download_url"] = "https://attacker.example/profile-release-selection.zip"

    assert RESOLVE._artifact_cohort(artifacts) is None


@pytest.mark.parametrize(
    "mutation",
    [
        {"schema": "wrong"},
        {"ok": False},
        {"channel": "../nightly"},
        {"profile": ""},
        {"profile_revision": ""},
        {"publication_identity": "profile-stable-code-2026.06.08.7"},
    ],
)
def test_selection_identity_rejects_malformed_or_inconsistent_reports(
    mutation: dict[str, object],
) -> None:
    document = _selection()
    document.update(mutation)

    with pytest.raises(ValueError):
        RESOLVE.selection_identity(document)


def test_cli_writes_only_the_reusable_run_id_to_github_output(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    selection_path = tmp_path / "selection.json"
    selection_path.write_text(json.dumps(_selection()), encoding="utf-8")
    output = tmp_path / "github-output"
    monkeypatch.setenv("GITHUB_TOKEN", "test-token")
    monkeypatch.setattr(RESOLVE, "find_reusable_run", lambda **_kwargs: 30185378359)

    assert (
        RESOLVE.main(
            [
                "--repository",
                "google/capsem",
                "--workflow",
                "release-assets.yaml",
                "--current-run-id",
                "99",
                "--selection",
                str(selection_path),
                "--github-output",
                str(output),
            ]
        )
        == 0
    )
    assert output.read_text(encoding="utf-8") == "run_id=30185378359\n"


def test_artifact_redirect_never_leaks_github_token_to_blob_storage() -> None:
    request = Request(
        "https://api.github.com/repos/google/capsem/actions/artifacts/1/zip",
        headers={"Authorization": "Bearer secret", "User-Agent": "test"},
    )
    redirected = RESOLVE.SafeArtifactRedirectHandler().redirect_request(
        request,
        None,
        302,
        "Found",
        {},
        "https://results.example.blob.core.windows.net/artifact.zip?signature=test",
    )

    assert redirected is not None
    assert redirected.get_header("Authorization") is None
    assert redirected.get_header("User-agent") == "test"
