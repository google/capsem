"""Exercise the publisher's actual retention step over paginated versions."""

import json
import os
import subprocess
import sys
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]


def test_retention_keeps_latest_tagged_generations_across_pages(tmp_path: Path) -> None:
    workflow = yaml.safe_load((ROOT / ".github/workflows/private-images.yaml").read_text())
    step = next(step for step in workflow["jobs"]["publish"]["steps"] if step.get("name", "").startswith("Keep the latest"))
    manifest = tmp_path / "cache/target/private-images/manifest.json"
    manifest.parent.mkdir(parents=True)
    manifest.write_text(json.dumps({"keep_generations": 2, "images": [{"package": "google/capsem-ci/family-linux-arm64"}] * 2}))
    versions = [{"id": i, "created_at": f"2026-01-0{i}", "metadata": {"container": {"tags": [] if i == 3 else [str(i)]}}} for i in range(1, 6)]
    (tmp_path / "versions.json").write_text(json.dumps([versions[:2], versions[2:]]))
    gh = tmp_path / "gh"
    gh.write_text(f"#!{sys.executable}\n" + '''
import pathlib, sys
args = sys.argv[1:]
# gh rejects this combination before any HTTP request (the hosted failure).
if '--slurp' in args and '--jq' in args:
    sys.exit('the --slurp option is not supported with --jq')
if '--method' in args:
    assert args[args.index('--method') + 1] == 'DELETE'
    with open('deleted.txt', 'a') as output: output.write(args[-1] + '\\n')
else:
    assert '--paginate' in args and '--slurp' in args
    assert '/capsem-ci%2Ffamily-linux-arm64/versions?' in ' '.join(args)
    print(pathlib.Path('versions.json').read_text())
''')
    gh.chmod(0o755)
    result = subprocess.run(["bash", "-c", step["run"]], cwd=tmp_path, env={**os.environ, "PATH": f"{tmp_path}:{os.environ['PATH']}", "OWNER": "google"}, capture_output=True, text=True, timeout=10, check=False)
    assert result.returncode == 0, result.stderr
    assert [int(path.rsplit("/", 1)[-1]) for path in (tmp_path / "deleted.txt").read_text().splitlines()] == [2, 1]
