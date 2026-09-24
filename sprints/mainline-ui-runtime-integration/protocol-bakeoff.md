# Protocol Bake-Off

## Rule

Do not build the production runtime until the protocol is selected.

This is the same discipline as the LLM library decision: inspect alternatives,
test the surface we actually need, discard what does not fit, and only then
implement our version.

Once selected, implement **one** shared protocol path. Do not support multiple
CRDT engines or renderer-specific state protocols in parallel.

## What The Protocol Must Carry

The protocol must support:

- UI surfaces and rich artifacts.
- Editable cards and component surfaces.
- A2UI Basic and Capsem extension components.
- Ordered replay and restart restore.
- Checkpoints/compaction.
- Comments/tasks on exact topology targets.
- Typed mutations for titles, text, props, style, chart config, slide regions,
  sheet cells, website/page/form nodes, and media metadata.
- Artifact references for image, video, audio, charts, diagrams, slides, and
  spreadsheets.
- Website/page/form artifacts with editable structure from the first version.
- Principal, role, title, timestamp, content type, verb, provenance, and result.
- Local MCP/tool calls and observations.
- Telemetry and cost metadata for generation/model calls.

## Candidate Protocol Shapes

| Candidate | Shape | What We Test | Likely Use |
| --- | --- | --- | --- |
| Capsem record protocol | Authoritative append-only records with typed content, projection, replay, and checkpoints | Security audit, deterministic replay, compact snapshots, task lifecycle, rich artifacts | Strong default for Capsem-controlled sessions |
| A2UI-native stream | A2UI `createSurface`, `updateComponents`, `updateDataModel`, `deleteSurface` as the primary protocol | Standards fit, component coverage, data binding, model friendliness | Good wire layer for UI construction, probably not enough for tasks/audit alone |
| A2UI plus Capsem envelope | Capsem record envelope carries A2UI messages plus task/artifact/provenance metadata | Best of standard UI payloads and Capsem audit/runtime needs | Leading candidate if tests pass |
| CRDT document protocol | Yrs/Yjs, Automerge, or Loro document update is the state protocol | Offline edits, concurrent comments/mutations, browser/Rust isomorphism, update size | Only if collaboration/offline beats audit complexity |
| MCP event log | MCP-style tool call/result stream as the primary protocol | Tool interoperability and agent ergonomics | Useful as tool boundary, probably not canonical UI state |

## Test Fixture

Every candidate must encode the same fixture:

1. Create a chat surface.
2. Add a Loro-backed card with image, title, text, actions, and link.
3. Add a Plotly-style bar chart artifact with two series.
4. Add a Mermaid diagram artifact.
5. Add a spreadsheet with one sheet and a few cells.
6. Add a slide deck with two slides, one embedding the chart.
7. Add a website page artifact with a form containing text input, checkbox,
   choice picker, submit action, and validation copy.
8. Add a generated image artifact with provider/cost provenance.
9. Add a user comment on the card title.
10. Mutate only the card title through Loro state.
11. Mutate one chart series color.
12. Mutate one slide region.
13. Mutate one sheet cell.
14. Mutate one website form label.
15. Resolve the comment.
16. Checkpoint/compact.
17. Replay after restart.

## Measurements

For each candidate, record:

- Bytes for initial fixture.
- Bytes for each mutation.
- Human readability for agent/debugging.
- Rust type safety.
- TS/Svelte parsing burden.
- Deterministic replay behavior.
- Checkpoint/compaction story.
- Topology addressing story.
- Comment/task lifecycle story.
- Principal/provenance/audit story.
- A2UI conformance story.
- Artifact extensibility.
- MCP/tool mapping.
- Security review complexity.

## Pass Criteria

A candidate can win only if:

- It can represent every fixture step without hidden side channels.
- Rust can validate it before Svelte sees it.
- It has a compact replay/checkpoint path.
- It preserves principal/provenance for every user/model/plugin change.
- `local.ui.info` can reconstruct topology and mutation affordances from it.
- It can reject unsupported components/mutations loudly.
- It can support A2UI Basic plus Capsem artifacts without protocol drift.

## Expected Decision Output

T0A produces:

- `selected-protocol.md`: chosen protocol and rejected alternatives.
- Fixture JSON/binary examples for each tested candidate.
- A comparison table with the measurements above.
- Minimal Rust/TS parser proof for the selected protocol only.
- Updated implementation tasks for T1 and later.
