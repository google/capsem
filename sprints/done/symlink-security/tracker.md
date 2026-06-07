# Sprint: Symlink Security Hardening

## Tasks

### Phase 1: Block symlink traversal in guest I/O (CRITICAL)
- [x] Add `validate_file_path_safe()` in `capsem-proto/src/lib.rs` with symlink + canonicalize + containment check
- [x] Add `O_NOFOLLOW` helpers (`write_nofollow`, `read_nofollow`, `delete_nofollow`) in capsem-agent
- [x] Update guest agent FileWrite handler -- was completely unvalidated
- [x] Update guest agent FileRead handler to use new validation + O_NOFOLLOW
- [x] Update guest agent FileDelete handler to use new validation + O_NOFOLLOW
- [x] Tests: 5 validate_file_path_safe tests in capsem-proto
- [x] Tests: 5 O_NOFOLLOW tests in capsem-agent

### Phase 2: Fix snapshot system (HIGH)
- [x] `collect_files()` -- use `WalkDir::follow_links(false)`, include symlinks
- [x] `workspace_hash()` -- include symlinks + hash link targets
- [x] `ReflinkSnapshot::snapshot()` -- preserve symlinks as symlinks
- [x] Compact/merge operation -- preserve symlinks
- [x] Snapshot file count -- include symlinks
- [x] Tests: 3 new auto_snapshot tests, 2 new file_tools tests

### Phase 3: Detection and reporting (MEDIUM)
- [x] Add `FileEntry` struct with `is_symlink` field
- [x] Surface `is_symlink` in `ChangedFile` and snapshot JSON output
- [x] Unskip `test_scenario_s21_symlink_revert` in test_mcp.py
- [x] CHANGELOG updated

## Notes
- Discovered 2026-04-07 during `just install` fix session
- The symlink escape in guest FileWrite/Read/Delete is the highest priority -- it's a direct sandbox escape
- `handle_revert_file()` already has a canonicalize check (file_tools.rs:598-609) -- the pattern exists, just not applied everywhere
- The `validate_file_path()` signature change (adding workspace_root) will touch capsem-proto and capsem-agent
