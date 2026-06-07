# Sprint T7: Snapshot Lifecycle Tests

## Goal

Validate the full snapshot lifecycle: automatic snapshots with interval-based ring buffers, manual named snapshots with integrity hashing, revert to previous state, history traversal across snapshots, and compaction with newest-file-wins semantics.

## Files

```
tests/capsem-snapshots/
    conftest.py
    test_auto_snapshots.py
    test_manual_snapshots.py
    test_revert.py
    test_history.py
    test_compact.py
```

Marker: `snapshot`

## Tasks

### Auto Snapshots (`test_auto_snapshots.py`)
- [ ] Verify auto snapshots fire at configured interval
- [ ] Verify ring buffer rotation evicts oldest when full
- [ ] Verify eviction removes snapshot data on disk
- [ ] Verify metadata.json written with timestamp, index, and source
- [ ] Verify file copy captures current workspace state accurately

### Manual Snapshots (`test_manual_snapshots.py`)
- [ ] Create a named snapshot and verify it appears in list
- [ ] Verify blake3 hash stored matches snapshot content
- [ ] Delete a named snapshot and verify removal from disk and index
- [ ] Exceed max slots and verify error returned
- [ ] List snapshots returns sorted by creation time (newest first)

### Revert (`test_revert.py`)
- [ ] Write file -> snapshot -> modify file -> revert -> file matches original
- [ ] Revert restores a file that was deleted after the snapshot
- [ ] Revert to nonexistent snapshot ID returns descriptive error

### History (`test_history.py`)
- [ ] Retrieve all versions of a file across multiple snapshots
- [ ] History includes entries for files that were later deleted

### Compact (`test_compact.py`)
- [ ] Compact merges snapshots with newest-file-wins semantics
- [ ] Source snapshots are deleted after successful compaction

### Infrastructure (`conftest.py`)
- [ ] Create shared fixture: booted VM with workspace and snapshot config
- [ ] Register `snapshot` pytest marker

## Verification

```bash
pytest tests/capsem-snapshots/ -m snapshot -v
```

All tests green. No snapshot data leaked between tests (each test gets a fresh VM or clean workspace).

## Depends On

None (standalone snapshot subsystem).
