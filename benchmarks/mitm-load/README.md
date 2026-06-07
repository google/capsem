# mitm-load baselines

Pre-rewrite baseline at `baseline.json` (run once with `just run
"capsem-bench mitm-load"` against the un-redesigned proxy). T5's CI
gate compares against this file: any concurrency level showing >2x p99
regression fails the build.

Refresh procedure: same command on a clean checkout, replace
`baseline.json`, commit with a CHANGELOG entry explaining the move.

Schema is documented inline in `guest/artifacts/capsem_bench/mitm_load.py`.
