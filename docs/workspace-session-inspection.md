# Workspace Session Inspection

Recorded on 2026-06-06 in the isolated UI/workspace prototype.

## Question

Can another agent audit a user-visible UI session without reading browser state
or trusting console output?

The inspection drives the Rust prototype over HTTP, records live telemetry,
stops the server, then inspects the SQLite workspace store directly. This is
still prototype evidence: mainline session DB telemetry remains a separate
integration gate.

## Command

```bash
npm run inspect:workspace
```

## Scenario

- Create a table artifact.
- Add a user change request with a concrete DOM annotation target.
- Apply a style patch tied to the request sequence.
- Apply a title patch.
- Record a renderer failure through `local.ui.renderError`.
- Resolve the task.
- Save a checkpoint.
- Inspect live telemetry and durable SQLite records/checkpoints.

## Result

```text
# Workspace Session Inspection

| Lane | Proof |
| --- | --- |
| live telemetry | 7 events: render.error, ui.table, workspace.create, workspace.patch, workspace.request, workspace.respond |
| workspace audit | workspace.create, workspace.patch, workspace.request, workspace.respond |
| render audit | inspect-house-table:preline-table |
| durable records | 5 records, max seq 5, verbs create, patch, request, respond |
| durable checkpoint | 1 checkpoint at seq 5 |
| durable principals | assistant.ui, chat.ui, local.ui.mutate, local.ui.table |
| durable content | action, artifact, elementPatch |
| mutation/task coverage | task=true, title=true, style=true, resolve=true |
```

## Conclusion

The isolated lane now has auditable session proof for the first production
shape: create, comment, mutate, render-error audit, resolve, checkpoint, and
direct SQLite record inspection.

This does not clear mainline telemetry. The prototype telemetry vector is still
in memory and intentionally disappears on process restart. Mainline integration
must wire the same event shape into Capsem session telemetry before release.
