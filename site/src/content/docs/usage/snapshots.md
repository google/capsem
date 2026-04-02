---
title: Snapshots
description: Use the snapshots CLI and MCP tools to checkpoint, diff, and restore workspace files inside a Capsem session.
sidebar:
  order: 1
---

Capsem automatically snapshots your workspace every 5 minutes. You can also create named checkpoints, view file history, and revert files -- from the terminal or through any MCP-aware AI agent.

## Quick reference

```
snapshots list                          # show all snapshots
snapshots create <name>                 # named checkpoint
snapshots changes                       # files changed since last snapshot
snapshots revert <path> [checkpoint]    # restore a file
snapshots history <path>                # file timeline across snapshots
snapshots delete <checkpoint>           # remove a manual snapshot
snapshots compact <cp1> <cp2> ...       # merge snapshots into one
```

All commands accept `--json` for machine-readable output.

## Listing snapshots

```
snapshots list
```

Shows all populated snapshot slots with checkpoint ID, origin (auto/manual), name, age, file count, and a change summary.

```
Snapshots (5 total, 10 manual slots available)
Checkpoint  Origin  Name             Age          Hash          Files  Changes
----------------------------------------------------------------------------
cp-3        auto    -                just now     -             20     +2, ~1
cp-2        auto    -                5 min ago    -             18     ~3, -1
cp-10       manual  before_refactor  8 min ago    a1b2c3d4e5f6  18     +1, ~2, -1
cp-1        auto    -                10 min ago   -             16     ~1
cp-0        auto    -                15 min ago   -             14     +14
```

## Creating checkpoints

```
snapshots create before_refactor
```

Creates a named manual snapshot. Names must be 1-64 characters (letters, digits, underscores, hyphens). If no name is given, a timestamp is used.

Manual snapshots are ideal before risky operations:

```
snapshots create before_db_migration
# ... run migration ...
# something went wrong?
snapshots revert schema.sql before_db_migration
```

## Reverting files

```
snapshots revert src/main.py            # from newest snapshot containing the file
snapshots revert src/main.py cp-3       # from a specific checkpoint
```

If the file existed in the snapshot, it is restored. If it didn't exist (was created after the snapshot), it is deleted from the workspace. Every revert is logged as a `restored` file event in the session database, including which checkpoint was used.

## Viewing file history

```
snapshots history src/main.py
```

Shows how a file changed across all snapshots -- creation, modification, deletion, and sizes at each checkpoint.

## Inspecting changes

```
snapshots changes
```

Lists all files that differ between the current workspace and the most recent snapshots. Shows which checkpoint last captured each file and what changed (new, modified, deleted).

## Deleting manual snapshots

```
snapshots delete cp-10
```

Removes a manual snapshot to free the slot. Auto snapshots cannot be deleted -- they are managed by the ring buffer.

## Compacting snapshots

```
snapshots compact cp-10 cp-11 --name merged
```

Merges multiple manual snapshots into one using newest-file-wins for conflicts. The merged result is placed in the lowest-numbered slot. Originals are removed.

## MCP tools

The same operations are available as MCP tools for AI agents. Any MCP-aware client running in the guest (Claude Code, Gemini CLI, etc.) can call them directly.

| MCP Tool | Arguments | Description |
|----------|-----------|-------------|
| `snapshots_list` | `format?`, `start_index?`, `max_length?` | List all snapshots |
| `snapshots_create` | `name` | Create named checkpoint |
| `snapshots_changes` | `format?`, `start_index?`, `max_length?` | List changed files |
| `snapshots_revert` | `path`, `checkpoint?` | Restore file from snapshot |
| `snapshots_history` | `path` | File version timeline |
| `snapshots_delete` | `checkpoint` | Delete manual snapshot |
| `snapshots_compact` | `checkpoints[]`, `name?` | Merge snapshots |

The `format` parameter accepts `"text"` (default, human-readable table) or `"json"` (structured). Pagination is supported via `start_index` and `max_length`.

## GUI

The Capsem GUI shows snapshot data in the **Stats > Snapshots** tab. The table updates each time you navigate to the tab, showing per-snapshot file change counts (created, modified, deleted) cross-referenced with the file event log.

## Configuration

Set these in `~/.capsem/user.toml`:

```toml
[vm.snapshots]
auto_max = 20         # default: 10
manual_max = 12       # default: 12
auto_interval = 120   # default: 300 (seconds)
```

For architecture details (storage layout, cloning backends, session DB schema), see [Snapshots Architecture](/architecture/snapshots/).
