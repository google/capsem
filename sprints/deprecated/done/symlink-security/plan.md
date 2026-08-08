# Sprint: Symlink Security Hardening

## Problem

Symlinks inside the guest VM are a sandbox escape vector. The guest agent follows symlinks when handling FileWrite/FileRead/FileDelete, and the snapshot system silently ignores them. An attacker can create a symlink pointing to a system file, then use MCP tools to read/write/delete that file through the symlink.

## Attack chain (proof of concept)

```bash
# Inside guest
ln -s /etc/hostname /root/hostname_backup

# From host via MCP write_file tool
write_file("/root/hostname_backup", "evil_hostname")
# Result: /etc/hostname overwritten via symlink traversal
```

`validate_file_path()` only checks for `..` and NUL bytes -- it passes `/root/hostname_backup` because the path looks clean. `std::fs::write()` follows the symlink.

## Scope

Full codebase audit found 12 symlink-related issues across 6 components:

### CRITICAL (sandbox escape)

| Component | File | Issue |
|-----------|------|-------|
| Guest FileWrite/Read/Delete | `capsem-agent/src/main.rs` ~698,718,732 | `std::fs::write/read/remove_file` follow symlinks; `validate_file_path()` doesn't check |
| Path validation | `capsem-proto/src/lib.rs:253-264` | Only checks `..` and NUL, not symlink resolution |
| Snapshot backend (Reflink) | `auto_snapshot.rs:560-616` | `ReflinkSnapshot::snapshot()` silently skips symlinks -- they persist after revert |
| Snapshot revert only | `file_tools.rs:598-609` | Symlink escape check exists in `handle_revert_file()` but NOT in FileRead/Write/Delete |

### HIGH (integrity)

| Component | File | Issue |
|-----------|------|-------|
| collect_files() | `file_tools.rs:311-327` | WalkDir follows symlinks -- snapshot diffs see target content, not the symlink |
| workspace_hash() | `auto_snapshot.rs:440-460` | Hash includes symlink-target sizes -- two different workspaces can produce same hash |

### MEDIUM (stealth/info)

| Component | File | Issue |
|-----------|------|-------|
| ApfsSnapshot | `auto_snapshot.rs:473-503` | clonefile(2) symlink behavior undocumented, `cp -R` fallback preserves them |
| Snapshot file count | `auto_snapshot.rs:165-169` | Symlinks excluded from count -- invisible in metadata |
| Snapshot listing | `file_tools.rs:780-820` | Symlinks invisible in MCP snapshot listing output |

### LOW (build-time / controlled)

| Component | File | Issue |
|-----------|------|-------|
| Asset hash | `asset_manager.rs:575-590` | `hash_file()` follows symlinks (assets are Capsem-controlled) |
| Builder copytree | `docker.py:708,717` | Preserves symlinks (correct, but worth documenting) |

## Approach

### Phase 1: Block symlink traversal in guest I/O (CRITICAL)

Fix `validate_file_path()` in `capsem-proto/src/lib.rs` to reject paths that are or contain symlinks:

```rust
pub fn validate_file_path(path: &str, workspace_root: &Path) -> Result<()> {
    // Existing checks
    if path.is_empty() { bail!("empty"); }
    if path.contains('\0') { bail!("NUL byte"); }
    if path.contains("..") { bail!("contains '..'"); }

    // Symlink check: resolve and verify it stays in workspace
    let full = workspace_root.join(path);
    if full.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) {
        bail!("path is a symlink");
    }
    // Check each component for symlink traversal
    let resolved = full.canonicalize()?;
    let ws_resolved = workspace_root.canonicalize()?;
    if !resolved.starts_with(&ws_resolved) {
        bail!("path resolves outside workspace");
    }
    Ok(())
}
```

Update callers in `capsem-agent/src/main.rs` to pass workspace root.

### Phase 2: Fix snapshot system (HIGH)

1. Use `WalkDir::new(root).follow_links(false)` in `collect_files()` and `workspace_hash()`
2. In `ReflinkSnapshot::snapshot()`, handle symlinks explicitly:
   - Option A: Preserve symlinks as symlinks (store the link target, not the content)
   - Option B: Reject/skip symlinks and log a warning
3. Add symlink count to snapshot metadata

### Phase 3: Detection and reporting (MEDIUM)

1. Add `is_symlink` flag to snapshot file listings
2. Add `symlinks_count` to snapshot info response
3. Add a doctor test: `test_no_symlinks_escape_workspace`

## Testing

- [ ] Test: FileWrite to a symlink path is rejected
- [ ] Test: FileRead through a symlink is rejected
- [ ] Test: FileDelete of a symlink is rejected
- [ ] Test: Symlink pointing outside workspace is rejected even if path looks clean
- [ ] Test: collect_files() does not follow symlinks
- [ ] Test: workspace_hash() does not follow symlinks
- [ ] Test: Snapshot captures symlinks as metadata, not targets
- [ ] Test: Revert restores symlinks as symlinks
- [ ] Doctor test: no symlinks escaping workspace root
