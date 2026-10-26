# Documentation metrics sources

`github.json` retains captured release-attempt and publication metadata so GitHub
retention does not erase the historical chart. The exporter refreshes unseen
attempts and releases through the read-only GitHub API when a token is supplied.

`benchmarks.json` retains the reviewed regular release recordings from Actions
artifact `11429814452`, source `f9c4ff3af881d86b50a518a7da3d77825ac4ffd0`.
Benchmark format versions inside records are distinct from release versions.
Exploratory `kingslanding` groups and local diagnostic runs are excluded. The
importer checks for a newer eligible hosted artifact; an expired artifact leaves
the retained recording intact. Raw logs, request bodies and credentials are not
part of these sources.

Generate the pages' downloadable data from the repository root:

```sh
uv run --project build_system --frozen --group docs python -m capsem_builder.docs_metrics
```

`GH_TOKEN` or `GITHUB_TOKEN` enables GitHub refresh. `--offline` keeps the retained
GitHub/benchmark evidence while still calculating source statistics from Git.
The source scanner is a pinned, checksum-verified scc binary. Source snapshots
follow original main through its explicit history-restoration merge and then
current main, at UTC month ends plus the actual initial commit and latest partial
month. Missing resolved lockfiles remain missing; test definitions are not
multiplied by architectures or parameterized cases.

The exporter writes `web/docs/public/data/metrics/`; those generated outputs are
rebuilt in docs CI. No VM or benchmark execution is part of this command.
