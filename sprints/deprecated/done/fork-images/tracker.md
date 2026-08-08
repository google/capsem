# Sprint: Fork & Images

## S1: Foundation -- ImageRegistry + session DB schema
- [x] `ImageEntry`, `ImageRegistry` in `capsem-core/src/image.rs`
- [x] `create_image_from_session()` with session.db checkpoint
- [x] `create_session_from_image()` (system + workspace clone, fresh telemetry)
- [x] `images_for_base_version()` genealogy query
- [x] Schema v5 migration: `source_image`, `persistent` columns
- [x] Populate `rootfs_version` and `rootfs_hash` in session creation
- [x] Extend `cleanup_old_versions()` with `&ImageRegistry` protection
- [x] Unit tests
- [x] Commit milestone

## S2: Service API -- fork + image CRUD + provision-from-image
- [x] Add `ImageRegistry` to `ServiceState`
- [x] API types: `ForkRequest/Response`, `ImageInfo`, `ImageListResponse`
- [x] Add `image: Option<String>` to `ProvisionRequest`
- [x] `handle_fork` handler
- [x] `handle_image_list`, `handle_image_inspect`, `handle_image_delete`
- [x] Modify `provision_sandbox()` for image-based boot
- [x] Populate new session fields (`rootfs_version`, `source_image`, `persistent`)
- [x] Vacuum respects `persistent` flag
- [x] Wire routes to Router
- [x] API serde roundtrip tests
- [x] Commit milestone

## S3: CLI + MCP
- [x] CLI: `capsem fork` command
- [x] CLI: `capsem image {list, delete, inspect}` subcommand group
- [x] CLI: `--image` flag on `capsem create`
- [x] MCP: `capsem_fork` tool
- [x] MCP: `capsem_image_list`, `capsem_image_delete`, `capsem_image_inspect` tools
- [x] MCP: add `image` param to `capsem_create`
- [x] Commit milestone

## S4: Integration tests + polish
- [x] Test: fork VM -> boot from image -> verify state (CLI + MCP)
- [x] Test: fork running VM vs stopped VM (CLI)
- [x] Test: image CRUD (list/delete/inspect) (CLI + MCP)
- [x] Test: error cases (duplicate name, missing VM, missing image) (CLI + MCP)
- [x] Test: asset cleanup protects referenced squashfs versions (Rust unit)
- [x] Test: vacuum skips persistent VM sessions (schema v5 migration)
- [x] Update existing tests: MCP discovery (21 tools), tool router, wait_exec_ready fix
- [x] Test: multiple VMs from same image (independence) (CLI)
- [x] Test: MCP fork tool discovery + schema validation
- [x] Rust unit tests: error paths (duplicate image, missing image, empty session)
- [x] Skills updated: dev-testing, dev-mcp, dev-capsem, site-architecture
- [x] CHANGELOG entries
- [ ] Commit milestone

## Notes
