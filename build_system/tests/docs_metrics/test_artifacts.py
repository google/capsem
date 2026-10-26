import io
import json
import zipfile

from capsem_builder.docs_metrics.artifacts import refresh, snapshot_from_archive
from capsem_builder.docs_metrics.github import GitHub


def test_import_uses_regular_json_and_excludes_exploratory_subdirectories():
    content = io.BytesIO()
    with zipfile.ZipFile(content, "w") as archive:
        archive.writestr(
            "capsem-bench/data_0.6.6_x86_64.json",
            json.dumps({"arch": "x86_64", "version": "0.3.0"}),
        )
        archive.writestr("kingslanding/spike/identity.json", json.dumps({"doctor_exit": 1}))
    artifact = {
        "id": 12,
        "created_at": "2026-10-06T00:00:00Z",
        "workflow_run": {"id": 5, "head_sha": "a" * 40},
    }
    data = snapshot_from_archive(content.getvalue(), artifact)
    assert data["source"]["release_label"] == "0.6.6"
    assert len(data["records"]) == 1


def test_expired_artifacts_do_not_erase_reviewed_recordings():
    client = GitHub(
        fetch=lambda _: {"artifacts": [{"expired": True, "created_at": "2026-10-10T00:00:00Z"}]}
    )
    retained = {"source": {"artifact_created_at_utc": "2026-10-06T00:00:00Z"}}
    assert refresh(retained, client) is retained
