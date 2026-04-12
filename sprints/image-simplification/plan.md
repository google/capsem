# Sprint: Image Simplification & Elimination

## Context

The current image system introduces a separate noun ("image") that users must learn alongside sandboxes. The mental model is confusing: `fork` creates an "image" from a sandbox, `image list/delete/inspect` manages them, `create --image` boots from one. Users expect everything to be a sandbox -- fork clones a sandbox, delete cleans up after itself.

**Goal:** Eliminate "image" as a user-facing concept. Everything is a sandbox. Fork creates a new (stopped) sandbox from an existing one. When the last sandbox referencing a snapshot is deleted, the snapshot is garbage-collected automatically.

## Architecture Simplification

### Current model

```
sandbox (running/stopped) --fork--> image (separate registry, separate storage)
image --create --image--> sandbox (clones image into new session)
```

Two registries (persistent_registry.json + image_registry.json), two storage trees (`sessions/` + `images/`), two sets of CRUD commands.

### Target model

```
sandbox --fork--> sandbox (stopped, persistent, marked as "forked from X")
sandbox --create --from--> sandbox (clones source sandbox's state)
```

One registry. One storage tree. Fork is just "create a persistent sandbox from another sandbox's state". The `--image` flag becomes `--from <sandbox-name>`.

### Garbage collection

When a sandbox is deleted, check if any other sandbox references it as a fork source. If none do, clean up its snapshot data. This is reference counting:

- `PersistentVmEntry` already has `source_image: Option<String>` -- rename to `forked_from: Option<String>`
- On delete: count how many entries have `forked_from == this_sandbox_name`
- If zero references and the sandbox is stopped: safe to fully remove
- If references exist: keep the snapshot data, mark as "archived" (no longer bootable but retains state for dependents), or warn the user

Simpler alternative: just let the user delete sandboxes freely. Forked sandboxes already have a full independent copy (APFS clone). There's no shared state to protect. The `forked_from` field is purely metadata (provenance tracking), not a live dependency.

**Recommendation:** Go with the simpler model. Fork makes an independent copy. Delete is always safe. `forked_from` is informational only. No garbage collection needed -- APFS clones are cheap and independent.

## Scope

### Files to modify

| File | Lines | What changes |
|------|-------|--------------|
| `crates/capsem-core/src/image.rs` | ~480 | **Delete entirely.** Fork logic moves into session/sandbox creation. |
| `crates/capsem-core/src/lib.rs` | 1 | Remove `pub mod image;` |
| `crates/capsem-core/src/asset_manager.rs` | ~20 | Remove image registry version-protection check (lines 839-864) |
| `crates/capsem-service/src/main.rs` | ~200 | Remove image handlers, simplify fork to create persistent sandbox, remove image registry from state |
| `crates/capsem-service/src/api.rs` | ~50 | Remove ForkRequest/Response, ImageInfo, ImageListResponse. Add `from` field to ProvisionRequest |
| `crates/capsem/src/main.rs` | ~80 | Remove `Image` subcommand + `ImageCommands` enum, change `fork` to output sandbox ID, rename `--image` to `--from` on create |
| `crates/capsem-mcp/src/main.rs` | ~80 | Remove capsem_image_* tools, update capsem_fork to return sandbox, update capsem_create for `--from` |
| `crates/capsem-proto/` | TBD | Check for image-related types |

### What fork becomes

```
capsem fork <source-id> <new-name>
```

1. Service looks up source sandbox (running or stopped persistent)
2. Creates new persistent sandbox entry with `forked_from: Some(source_name)`
3. APFS-clones source's `system/` and `workspace/` into new sandbox session dir
4. Returns new sandbox ID/name
5. New sandbox is stopped and persistent -- user can `capsem resume <new-name>` to boot it

This reuses the existing `create_image_from_session()` logic but targets the persistent sandbox storage instead of the images directory.

### What `create --from` becomes

```
capsem create --from <sandbox-name> [-n <new-name>]
```

1. Looks up source sandbox by name
2. Clones its session state into the new sandbox
3. Boots the new sandbox

This replaces `create --image`. The source is a sandbox name, not an image name.

### What gets deleted

- `~/.capsem/images/` directory and `image_registry.json`
- `ImageRegistry` struct and all methods
- `ImageEntry` struct
- `create_image_from_session()` and `create_session_from_image()` functions
- `/fork/{id}` HTTP endpoint (replaced with new fork-as-sandbox endpoint)
- `/images`, `/images/{name}` HTTP endpoints
- `capsem image list/delete/inspect` CLI commands
- `capsem_image_list/inspect/delete` MCP tools
- `capsem_fork` MCP tool (replaced with updated version)

## Phasing

### Phase 1: Fork-as-sandbox (core change)

- Modify `fork` to create a persistent stopped sandbox instead of an image
- Move APFS clone logic from `image.rs` into sandbox creation path
- Update `PersistentVmEntry` to track `forked_from`
- Update service fork handler
- Update CLI fork output (print sandbox name instead of image info)

### Phase 2: Replace `--image` with `--from`

- Rename `create --image` to `create --from`
- Source is now a sandbox name, resolved from persistent registry
- Remove image registry lookups from provision path
- Keep `--image` as hidden deprecated alias

### Phase 3: Remove image infrastructure

- Delete `crates/capsem-core/src/image.rs`
- Remove image registry from service state
- Remove image HTTP routes
- Remove image CLI commands
- Remove image MCP tools
- Remove `~/.capsem/images/` handling
- Clean up asset_manager version-protection

### Phase 4: Migration & cleanup

- One-time migration: convert existing images to persistent stopped sandboxes
- Remove image_registry.json after migration
- Update tests (image.rs has 10 tests that need to become sandbox-fork tests)
- Run full test suite

## Testing gate

```bash
cargo test -p capsem-core    # Unit tests (fork-as-sandbox)
cargo test -p capsem-service # Service handler tests
cargo test -p capsem         # CLI parsing tests
just smoke                   # Integration tests
just test                    # Full suite
```

Key scenarios to verify:
- `capsem fork <running-vm> my-fork` creates a stopped persistent sandbox
- `capsem create --from my-fork` boots a new sandbox from forked state
- `capsem delete my-fork` works (no cascade issues)
- `capsem list` shows forked sandboxes
- `capsem resume my-fork` boots the forked sandbox
- Old `--image` flag still works (hidden compat)

## Estimated scope

~730 lines removed across 5 files. ~200 lines of new fork-as-sandbox logic (mostly moved from image.rs). Net reduction: ~500 lines and one entire abstraction layer.
