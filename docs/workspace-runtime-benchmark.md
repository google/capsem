# Workspace Runtime Benchmark

Recorded on 2026-06-06 in the isolated UI/workspace prototype.

## Question

Is the Rust-owned workspace lane fast enough to keep as the shared path for
chat UI, local tools, generated artifacts, comments, mutations, replay, and
future plugin output?

This benchmark measures the prototype service path, not the eventual mainline
session store. Browser render timing includes page load plus Web Component
hydration for table, Plotly chart, and Mermaid diagram output.

## Benchmark

Command:

```bash
npm run bench:workspace
```

Default benchmark shape:

- Rust prototype server with temporary SQLite workspace.
- `24` table artifacts plus `1` Plotly bar chart and `1` Mermaid diagram.
- `30` samples for projection, snapshot, and mutation paths.
- `10` samples for checkpoint and websocket fanout paths.
- `3` process restart samples for restore.
- `5` Playwright browser render samples.
- `5` websocket clients for stream fanout.

All time values are milliseconds. `QPS` is normalized from mean sample time.

| Operation | Samples | Ops | Mean ms | p50 ms | p95 ms | QPS |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Projection GET | `30` | `30` | `1.090` | `1.089` | `1.173` | `917` |
| Snapshot GET | `30` | `30` | `1.832` | `1.824` | `1.881` | `546` |
| Title mutation POST | `30` | `30` | `0.599` | `0.599` | `0.694` | `1,669` |
| Checkpoint POST | `10` | `10` | `1.870` | `1.857` | `1.914` | `535` |
| Stream fanout, 5 clients | `10` | `10` | `1.756` | `1.699` | `1.999` | `570` |
| Restore + server start | `3` | `3` | `1442.7` | `1444.1` | `1545.6` | `1` |
| Browser render table/chart/diagram | `5` | `5` | `606.9` | `608.8` | `613.4` | `2` |

## Conclusion

The Rust projection/mutation/checkpoint lane is not the bottleneck in this
prototype: core API operations are sub-2ms p95 on a workspace with 26 rendered
artifacts. The expensive paths are process restart and browser rendering.

Mainline work should therefore focus performance attention on:

- avoiding unnecessary full browser/page rehydration;
- lazy-loading or splitting heavy Plotly/Mermaid chunks;
- separating pure restore timing from `cargo run`/process startup in the next
  benchmark once the mainline service lifecycle exists.
