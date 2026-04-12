# Sprint: Image Simplification & Elimination

**Status: DONE**

## Context

The current image system introduces a separate noun ("image") that users must learn alongside sandboxes. The mental model is confusing: `fork` creates an "image" from a sandbox, `image list/delete/inspect` manages them, `create --image` boots from one. Users expect everything to be a sandbox -- fork clones a sandbox, delete cleans up after itself.

**Goal:** Eliminate "image" as a user-facing concept. Everything is a sandbox. Fork creates a new (stopped) sandbox from an existing one.

## What was done

### Core change: fork creates a sandbox, not an image

- `handle_fork` rewritten to create a `PersistentVmEntry` instead of an `ImageEntry`
- New `clone_sandbox_state()` in `auto_snapshot.rs` handles fsync + APFS clone + guest/ layout + symlinks
- `PersistentVmEntry` gained `forked_from: Option<String>` and `description: Option<String>`
- `source_image` field removed everywhere, replaced by `forked_from` (with `alias = "source_image"` for backward compat on persistent_registry.json)

### `--image` replaced by `--from`

- `ProvisionRequest.image` -> `ProvisionRequest.from` (with `alias = "image"` for backward compat)
- CLI: `--image` -> `--from` (with `alias = "image"`)
- MCP: `CreateParams.image` -> `CreateParams.from`
- `provision_sandbox()` looks up persistent_registry instead of image_registry

### Image infrastructure removed

- Deleted `crates/capsem-core/src/image.rs` (~480 lines)
- Removed `ImageRegistry`, `ImageEntry`, `ImageRegistryData` structs
- Removed `create_image_from_session()`, `create_session_from_image()` functions
- Removed `handle_image_list`, `handle_image_inspect`, `handle_image_delete` handlers + routes
- Removed `validate_image_name()` function
- Removed `ImageCommands` CLI enum + handlers
- Removed `capsem_image_list`, `capsem_image_inspect`, `capsem_image_delete` MCP tools
- Removed `ImageInfo`, `ImageListResponse`, `ImageNameParams` API types
- Removed `image_registry` from `ServiceState`

### Session DB schema v5 -> v6

- `source_image TEXT` column renamed to `forked_from TEXT`
- Migration chain updated for v2->v6, v3->v6, v4->v6, v5->v6

### API surface updates

- `SandboxInfo` gained `forked_from` and `description` fields
- `forked_from` populated for both running and stopped sandboxes
- `description` populated for stopped/suspended sandboxes (from `PersistentVmEntry`)

### cleanup_old_versions signature change

- `image_registry: Option<&ImageRegistry>` -> `extra_pinned: &[String]`
- All callers updated (capsem-app, capsem CLI update)

### No migration

Existing images in `~/.capsem/images/` are not migrated. Users must re-fork from running sandboxes.

## Files changed

| File | Change |
|------|--------|
| `crates/capsem-core/src/auto_snapshot.rs` | Added `clone_sandbox_state()` + 3 tests |
| `crates/capsem-core/src/image.rs` | Deleted entirely |
| `crates/capsem-core/src/lib.rs` | Removed `pub mod image` |
| `crates/capsem-core/src/asset_manager.rs` | `cleanup_old_versions` signature change |
| `crates/capsem-core/src/session/types.rs` | `source_image` -> `forked_from` |
| `crates/capsem-core/src/session/index.rs` | Schema v5->v6 migration |
| `crates/capsem-core/src/session/mod.rs` | Updated tests |
| `crates/capsem-service/src/main.rs` | Rewrite fork, remove image infra, add forked_from/description |
| `crates/capsem-service/src/api.rs` | Remove image types, add forked_from/description to SandboxInfo |
| `crates/capsem/src/main.rs` | Remove image commands, `--image` -> `--from` |
| `crates/capsem/src/client.rs` | Remove image types, add forked_from/description |
| `crates/capsem/src/update.rs` | `cleanup_old_versions` call updated |
| `crates/capsem-mcp/src/main.rs` | Remove image tools, update fork/create |
| `crates/capsem-app/src/main.rs` | `source_image` -> `forked_from` |
| `crates/capsem-app/src/cli.rs` | `source_image` -> `forked_from` |
| `frontend/src/lib/types/gateway.ts` | Remove ImageInfo, `image` -> `from` |

## Test results

- capsem-core: 1425 passed (1 pre-existing network failure)
- capsem-service: 76 passed
- capsem CLI: 135 passed
- capsem-mcp: 67 passed
