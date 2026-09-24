# Workspace Stream Projection Sub-Sprint

## Purpose

Build the append-only workspace lane that replaces direct UI writes. Generated images, future plugin output, model tool calls, and deck artifacts must enter the UI as records, pass through a Rust projection engine, and render through keyed Svelte maps.

## Contract Decisions

- A workspace item is an immutable `WorkspaceRecord` with `seq`, `id`, `timestamp`, `role`, `principal`, `title`, `contentType`, `verb`, `status`, `target`, and strongly typed `content`.
- `contentType` describes the payload family only. It is not a mixed semantic bucket.
- `principal` is a plain string because it names the exact actor: `codex`, `local.generate.image`, `plugin.git-context`, etc.
- The server owns replay, checkpoints, and render projection. Svelte receives `RenderDelta` operations and keeps `Map<ElementId, UiElement>` plus a topology graph: `roots[]` and `nodes{id,parent,slot,index}`.
- The UI does not write components directly. It applies deltas emitted by Rust, then renders keyed elements.
- User interactions also enter Rust as workspace records. Clicking an element posts a durable select record; asking for a change posts a `ui.change` action request targeted at that topology element.
- Annotation is more precise than element selection. A change request carries a generic `annotation { kind, label, path, selector, hostSelector, shadowSelector, selectorVerified, metadata }` derived from the actual DOM element clicked. Shadow DOM annotations use a stable Capsem host selector plus a numbered inner selector because plain `document.querySelector()` cannot cross that boundary.
- Checkpoints collapse prior records into current projection so deleted/replaced/transient items do not clutter the active surface. Local browser storage keeps the latest checkpoint pointer and cached projection.
- `RenderElement` carries durable provenance (`createdBy`, `updatedBy`, record ids, seqs, verb) so `local.ui.info()` can tell an agent what exists and what generated or last edited it.

## Files

- `crates/capsem-ui-catalog/src/workspace.rs` - record, checkpoint, projection, delta, reducer.
- `crates/capsem-ui-catalog/src/lib.rs` - export workspace module.
- `crates/capsem-ui-catalog/tests/workspace.rs` - reducer and compaction contract tests.
- `crates/capsem-plugin-server/src/main.rs` - publish artifact records, expose snapshot/replay/WebSocket stream.
- `ui-preview/src/nativeDeck.ts` - workspace stream types, snapshot parser, local cache helpers.
- `ui-preview/src/ChatPage.svelte` - keyed projection UI driven by WebSocket deltas.
- `test/native-workspace.test.ts` - frontend parser/cache contract tests.

## Done

- `POST /native/generate/image` stores an artifact and publishes a workspace record.
- `/native/workspace/snapshot` returns checkpoint + current render projection.
- `/native/workspace/stream` sends snapshot and then record frames with Rust-produced deltas.
- `/chat` uses keyed maps so an image card appears or replaces by artifact id without rebuilding the page list.
- `/native/workspace/info` exposes the current topology and provenance for tool-driven live editing.
- `/native/workspace/change-request` records UI-originated edit intents without mutating the selected element directly.
- `/native/workspace/style` records selector-scoped style mutations as `elementPatch` records, projects them into artifact specs as `stylePatches`, and lets Capsem elements apply whitelisted properties to exact Shadow DOM selectors.
- Shadow DOM artifact components install one generic annotator gated by Comment mode. The host is marked with `data-capsem-artifact-id`, every shadow element is numbered with `data-capsem-node`, and `/chat` opens an inspector overlay plus persistent comment panel next to the selected concrete element with traversal, dimensions, and computed-style metadata.
- Commenting is a user tool, not default click behavior. The floating Comment button arms selector capture; after a target is selected, the feedback panel stays visible with pending comments so the user can keep adding notes while the AI addresses the change. Submitting feedback leaves a pinned marker on the selected element; repeated feedback for the same selector increments the marker count, and clicking the marker reopens the thread.
- `/chat` reconnects dropped workspace streams and receives a fresh snapshot before new deltas, so visible UI mutation is not dependent on manual refresh after a server restart.
- Tests cover reducer behavior and frontend contract parsing.

## Coverage Matrix

- Unit/contract: Rust reducer tests, TS parser/cache tests.
- Functional: image generation endpoint publishes projection records.
- Adversarial: malformed frontend workspace shapes are rejected.
- E2E/browser: run server, generate image through API, verify `/chat` renders generated-image card.
- Telemetry: existing generation telemetry remains populated.
- Performance: deferred; this sprint is correctness of stream/projection shape.
