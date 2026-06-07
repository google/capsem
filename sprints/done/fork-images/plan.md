# Sprint: Fork & Images

## Context

Capsem VMs boot from a shared read-only `rootfs.squashfs` (base image) with a per-session overlayfs upper layer (`system/rootfs.img`, sparse 2GB ext4). Today, every VM starts from the same base -- there's no way to save a configured VM and reuse it as a starting point. The fork sprint adds **user images** as first-class objects: fork a VM's state into a reusable image, then boot new VMs from it.

This is the Docker `commit` equivalent -- "I installed Python, configured the environment, now save this as a template."

## Architecture

### What is an image?

An APFS-cloned copy of a VM's session directory, stored separately from running/persistent VMs:

```
~/.capsem/images/
    image_registry.json          # Index (same pattern as persistent_registry.json)
    python-dev/
        metadata.json            # Name, source VM, base_version, created_at, size
        system/
            rootfs.img           # APFS clone of the source VM's overlay
        workspace/               # APFS clone of workspace files
        session.db(.gz)          # Telemetry history from source VM
        serial.log               # Boot/runtime logs from source VM
        process.log              # Process logs from source VM
        auto_snapshots/          # Snapshot history from source VM
    node-setup/
        ...
```

Fork clones the **entire session directory** -- full VM state plus history:
- `system/` (rootfs.img overlay)
- `workspace/` (user files)
- `session.db` / `session.db.gz` (telemetry: network events, tool calls, file events)
- `serial.log`, `process.log` (VM boot + runtime logs)
- `auto_snapshots/` (snapshot ring buffer + named snapshots)

For running VMs: checkpoint session.db (WAL flush via `PRAGMA wal_checkpoint(TRUNCATE)`) before cloning to ensure the DB copy is clean.

**Why APFS clone?** Near-instant regardless of image size. Copy-on-write at filesystem level. Already proven in the auto-snapshot system (`clone_directory` in `auto_snapshot.rs`). No guest-side changes needed -- the boot flow already handles pre-formatted rootfs.img (persistent VM resume uses exactly this path).

### Boot from image flow

1. Service looks up image in `ImageRegistry`
2. Creates new session dir, APFS-clones image's `system/` and `workspace/` into it
3. Creates empty `auto_snapshots/` (fresh snapshot history for the new VM)
4. Resolves `rootfs.squashfs` from image's `base_version` (for the overlayfs lower layer)
5. Spawns `capsem-process` with `--rootfs {squashfs} --session-dir {new_session}` -- identical to normal boot
6. Guest finds already-formatted rootfs.img, skips mke2fs, mounts overlayfs with pre-existing changes

**No changes needed in capsem-process or capsem-init.** The session dir layout is identical to what persistent VM resume already produces.

### Fork consistency (running VMs)

Fork works on both running and stopped VMs. For running VMs, APFS clonefile is atomic at the filesystem block level. The guest's ext4 journal handles crash consistency (same guarantee as auto-snapshots of running VMs, which work today). Session.db is checkpointed before cloning.

## Image Genealogy Model

**The dependency graph is always flat: every fork depends only on a base squashfs, never on another fork.**

When you fork a VM, the cloned `rootfs.img` contains the complete overlay diff from the base squashfs. If you boot from fork A, install more, and fork into B -- B's rootfs.img has everything (A's changes + new changes). B does not reference A. Both A and B only need the same base squashfs.

```
rootfs.squashfs v0.16.1  <--  python-dev (fork)
                          <--  node-setup (fork)
                          <--  ml-env     (fork, originally booted from python-dev)

rootfs.squashfs v0.17.0  <--  new-base   (fork)
```

**Consequences:**
- Deleting any forked image is always safe -- no other image depends on it
- Deleting a base squashfs is dangerous -- must check no images reference it
- `cleanup_old_versions()` in `asset_manager.rs:638` must be extended to protect squashfs versions with dependent images
- The existing `pinned.json` mechanism can be used: when an image is created, auto-pin its `base_version`; when all images on that version are deleted, unpin it

**Fork from fork tracking:** Each image records `source_vm` (what VM it was forked from) and optionally `parent_image` (if that VM was booted from an image). This is informational genealogy -- not a hard dependency. The only hard dependency is `base_version`.

## API Design

### Service Routes

| Route | Method | Handler | Body |
|-------|--------|---------|------|
| `/fork/{id}` | POST | `handle_fork` | `ForkRequest` |
| `/images` | GET | `handle_image_list` | -- |
| `/image/{name}` | GET | `handle_image_inspect` | -- |
| `/image/{name}` | DELETE | `handle_image_delete` | -- |

### API Types (`capsem-service/src/api.rs`)

```rust
// New types
struct ForkRequest {
    name: String,                    // Image name (validated like VM names)
    description: Option<String>,     // What's in this image
}
struct ForkResponse { name: String, size_bytes: u64 }
struct ImageInfo { name, description, source_vm, parent_image, base_version, created_at, size_bytes }
struct ImageListResponse { images: Vec<ImageInfo> }

// Modified
struct ProvisionRequest {
    ...existing fields...
    image: Option<String>,           // Boot from user image instead of base
}
```

### CLI Commands (`capsem/src/main.rs`)

```
capsem fork <id|name> --name <image-name> [--description <text>]
capsem image list                    (alias: ls)
capsem image delete <name>           (alias: rm)
capsem image inspect <name>
capsem create -n myvm --image python-dev    (new --image flag)
```

### MCP Tools (`capsem-mcp/src/main.rs`)

| Tool | Params | Maps to |
|------|--------|---------|
| `capsem_fork` | id, name, description? | POST /fork/{id} |
| `capsem_image_list` | -- | GET /images |
| `capsem_image_delete` | name | DELETE /image/{name} |
| `capsem_image_inspect` | name | GET /image/{name} |
| `capsem_create` (modified) | +image? | POST /provision |

## ImageRegistry (capsem-core)

Self-contained image module in `capsem-core/src/image.rs`:

```rust
struct ImageEntry {
    name: String,
    description: Option<String>,
    source_vm: String,              // Name/ID of source VM
    parent_image: Option<String>,   // If source VM was booted from an image
    base_version: String,           // e.g. "0.16.1" -- HARD dependency on squashfs
    created_at: String,             // ISO 8601
    arch: String,                   // "arm64" / "x86_64"
    size_bytes: u64,                // Apparent size of image dir
    image_dir: PathBuf,             // ~/.capsem/images/{name}/
}

struct ImageRegistryData { images: HashMap<String, ImageEntry> }
struct ImageRegistry { path: PathBuf, data: ImageRegistryData }
// Methods: load(), save(), register(), unregister(), get(), list(), contains()
// Key method: images_for_base_version(&str) -> Vec<&ImageEntry>
//   Returns all images depending on a given squashfs version.
//   Used by asset cleanup to protect referenced versions.
```

**Where it lives:** `capsem-core` (shared library). The registry file is at `~/.capsem/images/image_registry.json`.

**Asset protection integration:** `cleanup_old_versions()` gains an optional `&ImageRegistry` parameter. Before removing a version directory, it checks `images_for_base_version()` -- if any images depend on it, the version is kept (treated as implicitly pinned).

## Core Functions (capsem-core)

Two new functions in `crates/capsem-core/src/image.rs`:

1. **`create_image_from_session(session_dir, image_dir, name, db_writer)`**
   - If VM running: checkpoint session.db via `PRAGMA wal_checkpoint(TRUNCATE)`
   - `clone_directory(session_dir, image_dir)` -- clones entire session (system, workspace, session.db, logs, auto_snapshots)
   - Write `metadata.json` into image_dir
   - Return `ImageEntry`

2. **`create_session_from_image(image_dir, session_dir)`**
   - `clone_directory(image_dir/system, session_dir/system)`
   - `clone_directory(image_dir/workspace, session_dir/workspace)`
   - Create fresh `auto_snapshots/` (new VM gets its own snapshot history)
   - Does NOT clone session.db/logs from image (new VM starts fresh telemetry)
   - Does NOT create blank rootfs.img (the cloned one is already formatted)

Both reuse `clone_directory` from `auto_snapshot.rs`.

## Session DB Integration

### Current state (problems to fix)
- `SessionRecord` has `rootfs_version` and `rootfs_hash` fields (added in schema v4) but they are **never populated** -- always `None`
- `persistent` flag lives only in `PersistentRegistry` (JSON), not in main.db. The vacuum system has no idea which sessions belong to persistent VMs.
- Vacuum (`session_mgmt.rs`) does age/count/disk culling blindly -- it could terminate a persistent VM's session.

### Changes needed

**Populate rootfs tracking in SessionRecord:**
- Set `rootfs_version` and `rootfs_hash` when creating sessions (service knows the version from AssetManager)
- This is the ground truth for "which squashfs does this session depend on"

**Add `source_image` field to SessionRecord** (schema v5 migration):
- `source_image: Option<String>` -- the image name if this VM was booted from a forked image
- Informational: lets us trace lineage. Not a hard dependency (the session has its own rootfs.img clone).

**Add `persistent` field to SessionRecord** (schema v5 migration):
- `persistent: bool` -- whether this is a named persistent VM
- Lets vacuum distinguish ephemeral (can auto-terminate) from persistent (must preserve)
- Currently vacuum could accidentally cull a stopped persistent VM's session

**Vacuum rules:**
- Ephemeral sessions: vacuum + terminate normally (unchanged)
- Persistent VM sessions: never auto-terminate while the VM is registered in PersistentRegistry. Only explicit `capsem delete` removes them.
- Image-based VMs: vacuum the session normally (the image is the template, the session is disposable). The image itself is never vacuumed -- it's not a session.

**Images carry full history but are NOT sessions:**
- Images live in `~/.capsem/images/`, not `~/.capsem/sessions/`
- Images are not tracked in main.db (no lifecycle state machine -- they're templates, not running instances)
- When forking: clone the entire session directory (system/, workspace/, session.db, logs, auto_snapshots/) -- all of this is valuable for UI display and forensic analysis
- For running VMs: checkpoint session.db before cloning (`PRAGMA wal_checkpoint(TRUNCATE)`)

**Asset cleanup integration:**
- `cleanup_old_versions()` must check both:
  1. Running VMs using that squashfs version (via main.db `rootfs_version` WHERE status='running')
  2. Images referencing that version (via `ImageRegistry.images_for_base_version()`)
- If either has references, the version directory is preserved

## Sprint Phasing

### S1: Foundation -- ImageRegistry + session DB schema
- `ImageEntry`, `ImageRegistry` in `capsem-core/src/image.rs`
- `create_image_from_session()` -- checkpoints session.db (if running), clones entire session dir into image dir
- `create_session_from_image()` -- clones image's system/ + workspace/ into new session dir (fresh telemetry + snapshots)
- `images_for_base_version()` query for genealogy tracking
- Schema v5 migration: add `source_image` and `persistent` columns to SessionRecord
- Populate `rootfs_version` and `rootfs_hash` during session creation (currently always None)
- Extend `cleanup_old_versions()` in `asset_manager.rs` to accept `&ImageRegistry` and protect referenced base versions
- Unit tests for registry CRUD, clone operations, genealogy queries, asset protection, schema migration
- **Commit milestone**: core image layer + schema changes compile and tests pass

### S2: Service API -- fork + image CRUD + provision-from-image
- Add `ImageRegistry` to `ServiceState`
- `handle_fork`: validate name, locate session_dir (running or stopped), clone, register
- `handle_image_list`, `handle_image_inspect`, `handle_image_delete`
- Modify `provision_sandbox()`: when `image` is set, use `create_session_from_image()` instead of `create_virtiofs_session()`
- Populate `rootfs_version`, `rootfs_hash`, `source_image`, `persistent` fields in new sessions
- Make vacuum respect `persistent` flag (never auto-terminate persistent VM sessions)
- Add routes to Router
- API type tests (serde roundtrip)
- **Commit milestone**: service compiles, fork + create-from-image work via curl

### S3: CLI + MCP
- CLI: `Fork` command, `Image { List, Delete, Inspect }` subcommand group, `--image` on Create
- MCP: `capsem_fork`, `capsem_image_list`, `capsem_image_delete`, `capsem_image_inspect`, add `image` param to `capsem_create`
- **Commit milestone**: full user-facing API works

### S4: Integration tests + polish
- Test: fork a VM, boot from image, verify state carried over
- Test: fork running VM, fork stopped VM
- Test: image list/delete/inspect
- Test: error cases (nonexistent VM, duplicate image name, missing image)
- Test: asset cleanup protects squashfs versions referenced by images
- Test: vacuum skips persistent VM sessions
- Update tests in `tests/capsem-service/`, `tests/capsem-cli/`
- CHANGELOG entries
- **Commit milestone**: all tests pass, sprint complete

## Files to Modify

| File | Change |
|------|--------|
| `crates/capsem-core/src/image.rs` | **NEW** -- ImageRegistry, ImageEntry, create/clone functions, genealogy queries |
| `crates/capsem-core/src/lib.rs` | Register `image` module, export public types |
| `crates/capsem-core/src/session/types.rs` | Add `source_image`, `persistent` fields to SessionRecord |
| `crates/capsem-core/src/session/index.rs` | Schema v5 migration, populate rootfs_version/rootfs_hash |
| `crates/capsem-core/src/asset_manager.rs` | Extend `cleanup_old_versions()` to protect base versions referenced by images + running VMs |
| `crates/capsem-service/src/api.rs` | ForkRequest/Response, ImageInfo, ImageListResponse, add `image` to ProvisionRequest |
| `crates/capsem-service/src/main.rs` | ImageRegistry in state, fork/image handlers, provision-from-image, populate new session fields, vacuum-respects-persistent |
| `crates/capsem/src/main.rs` | Fork command, Image subcommands, --image flag on Create |
| `crates/capsem-mcp/src/main.rs` | 4 new MCP tools + image param on capsem_create |
| `CHANGELOG.md` | Sprint entries |

## Reuse Points

- `clone_directory()` from `auto_snapshot.rs` -- APFS clonefile with fallback
- `PersistentRegistry` pattern from `capsem-service/src/main.rs:47-91` -- template for ImageRegistry
- `validate_vm_name()` from `capsem-service/src/main.rs` -- reuse for image name validation
- `create_virtiofs_session()` from `capsem-core/src/lib.rs` -- reference for session dir layout
- `vacuum_and_compress_session_db()` from `capsem-core/src/session/maintenance.rs` -- checkpoint pattern
- Existing `ProvisionRequest`/`ProvisionResponse` patterns for API types

## Verification

1. `just test` -- unit tests pass (registry CRUD, serde roundtrip, clone operations)
2. Manual: `capsem create -n mydev` -> install packages -> `capsem fork mydev --name python-env` -> `capsem create -n test --image python-env` -> verify packages present
3. Manual: `capsem image list` shows image, `capsem image inspect python-env` shows metadata, includes session history
4. Manual: MCP tools work via capsem-mcp
5. Manual: upgrade base version, verify old squashfs preserved if images depend on it
6. `just run "capsem-doctor"` -- VM smoke test still passes

## Not in scope (future work)

- Remote image registry (push/pull to share images)
- Image tagging/versioning (`name:tag`)
- Image rebasing (update an image's base squashfs to a newer version)
- Image layers (Docker-style shared base layers)
- Cross-machine portability
