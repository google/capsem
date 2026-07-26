from __future__ import annotations

import json
from pathlib import Path
import subprocess


ROOT = Path(__file__).resolve().parents[2]
GRAPH_FIXTURE = ROOT / "tests" / "capsem-release" / "fixtures" / "release-graph-stable-nightly.json"


def test_release_rebases_collision_free_root_payload_names(tmp_path: Path) -> None:
    graph = json.loads(GRAPH_FIXTURE.read_text(encoding="utf-8"))
    base = graph["manifests"]["nightly"]["1.0.2"]
    candidate = json.loads(json.dumps(base))
    profile = candidate["profiles"]["code"]
    profile["revision"] = "2026.07.26.1"
    profile["version"] = "2026.07.26.1"
    config = profile["architectures"][0]["config"]
    config.extend(
        [
            {
                "kind": "root_payload",
                "path": "profiles/code/root/root/.claude/settings.json",
                "url": ("/profiles/releases/nightly/code/2026.07.26.1/arm64/root-payload-aaaaaaaa"),
                "bytes": 10,
                "digest": {
                    "sha256": "a" * 64,
                    "blake3": "a" * 64,
                },
                "status": "current",
            },
            {
                "kind": "root_payload",
                "path": "profiles/code/root/root/.gemini/settings.json",
                "url": ("/profiles/releases/nightly/code/2026.07.26.1/arm64/root-payload-bbbbbbbb"),
                "bytes": 11,
                "digest": {
                    "sha256": "b" * 64,
                    "blake3": "b" * 64,
                },
                "status": "current",
            },
        ]
    )
    base_path = tmp_path / "base.json"
    candidate_path = tmp_path / "candidate.json"
    base_path.write_text(json.dumps(base), encoding="utf-8")
    candidate_path.write_text(json.dumps(candidate), encoding="utf-8")
    publication_base = (
        "https://github.com/google/capsem/releases/download/profile-nightly-code-2026.07.26.1"
    )

    result = subprocess.run(
        [
            "cargo",
            "run",
            "--quiet",
            "-p",
            "capsem-admin",
            "--",
            "release",
            "--channel",
            "nightly",
            "--profile",
            "code",
            "--manifest-path",
            str(base_path),
            "--candidate-manifest",
            str(candidate_path),
            "--publication-base",
            publication_base,
            "--manifest-version",
            "1.0.2",
            "--profile-version",
            "2026.07.26.1",
            "--json",
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )

    assert result.returncode == 0, result.stderr
    merged = json.loads(base_path.read_text(encoding="utf-8"))
    root_payloads = [
        row
        for row in merged["profiles"]["code"]["architectures"][0]["config"]
        if row["kind"] == "root_payload"
    ]
    assert [row["url"] for row in root_payloads] == [
        f"{publication_base}/arm64-root-payload-aaaaaaaa",
        f"{publication_base}/arm64-root-payload-bbbbbbbb",
    ]
