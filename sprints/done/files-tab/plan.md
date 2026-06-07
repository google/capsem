# Sprint: Files Tab -- Browse, Upload, Download

Make the Files tab in the VM dashboard fully functional: proper directory listing, copy-to-clipboard, file download, and drag-and-drop upload.

## Problem

The Files tab exists (FilesView + FileTree + FileContent) but is broken:

1. **File tree uses `find /workspace` via `execCommand`** -- goes through vsock, returns flat text with no metadata. The `buildTreeFromPaths()` in FilesView.svelte:63 uses a broken heuristic (`const isDir = !p.includes('.')`) that misidentifies directories with dots as files and extensionless files as directories.
2. **File reads go through vsock** -- 256KB frame limit (`MAX_FRAME_SIZE` in capsem-proto), content JSON-wrapped as UTF-8 string. Binary files get lossy conversion. Large files silently fail.
3. **No copy, download, or upload** in the UI. `writeFile()` API exists but is never called from any component.

## Approach

**Bypass vsock. Use host-side VirtioFS access.**

The VM workspace is shared via VirtioFS:
- **Host side**: `session_dir/guest/workspace/` (symlinked from `session_dir/workspace/`)
- **Guest side**: `/workspace` (VirtioFS tag `"capsem"`)
- Changes are immediately visible on both sides

The capsem-service already knows `session_dir` for every running VM (in `InstanceInfo`) and every stopped persistent VM (in `PersistentRegistry`). Three new endpoints read/write the host-side path directly -- no vsock, no frame limits, binary-safe, up to 10MB (matching the gateway proxy body limit).

**File type detection with Magika** (`magika` crate on crates.io). Google's AI-powered file identification from content bytes. Returns MIME type, label (e.g., "rust", "python"), and is_text flag. Used in list and download endpoints.

### Magika Implementation Notes

**Crate**: `magika` (latest on crates.io). Depends on `ort` (ONNX Runtime) -- the model is embedded via `include_bytes!("model.onnx")`, no external model files needed.

**Core types**:
```rust
use magika::{Session, FileType, ContentType, TypeInfo};
```

**Initialization** -- create once, reuse. Store in `ServiceState`:
```rust
// Session::new() loads the embedded ONNX model. Do this once at startup.
let magika = magika::Session::new().expect("failed to init magika");

// Or with tuning:
let magika = magika::Session::builder()
    .with_inter_threads(1)   // threads for graph parallelism
    .with_intra_threads(1)   // threads within nodes
    .build()
    .expect("failed to init magika");
```

**Identify from file path** (for list endpoint):
```rust
let file_type: FileType = magika.identify_file_sync("/path/to/file")?;
```

**Identify from bytes** (for download endpoint / in-memory content):
```rust
let file_type: FileType = magika.identify_content_sync(content_bytes)?;
```

**Extract results from FileType**:
```rust
match file_type {
    FileType::Inferred(inferred) => {
        let info: &TypeInfo = inferred.content_type.info();
        // TypeInfo fields:
        //   info.label       -> "rust", "python", "png", "pdf", etc.
        //   info.mime_type   -> "text/x-rust", "image/png", etc.
        //   info.group       -> "code", "image", "archive", "video", etc.
        //   info.description -> "Rust source"
        //   info.extensions  -> &["rs"]
        //   info.is_text     -> true/false
        let score: f32 = inferred.score;  // confidence 0.0-1.0
    }
    FileType::Ruled(content_type) => {
        // Rule-based (high confidence, no AI needed)
        let info: &TypeInfo = content_type.info();
        // Same TypeInfo fields available
    }
}
```

**Key details**:
- `Session` is `&mut self` for identify methods -- needs `Mutex<Session>` in `ServiceState`
- Model is ~1MB embedded in the binary, loads in <100ms
- `identify_file_sync` reads the file internally (first/last bytes + size)
- 217+ file types supported across code, documents, images, archives, etc.
- Async variants available: `identify_file_async`, `identify_content_async`
- Batch API: `identify_features_batch_sync` for bulk identification

**Pure Svelte + Preline CSS** for all new frontend components. The project uses Preline CSS-only (theme + variants) -- no Preline JS plugins are loaded. HSTreeView, HSFileUpload, HSCopyMarkup were evaluated and rejected: HSTreeView fights Svelte reactivity (DOM-based data attributes), HSFileUpload pulls in Dropzone.js + lodash, HSCopyMarkup clones DOM elements rather than copying text to clipboard.

## Security: Two-Layer Path Sanitization

Every path parameter goes through two checks:

### Layer 1: Allowlist sanitization (`sanitize_file_path`)
- Strip any character NOT in `[a-zA-Z0-9._\-/]`
- Collapse consecutive slashes (`//` -> `/`)
- Strip leading `/` (paths are relative to workspace root)
- Reject paths containing `..` after sanitization (400 Bad Request)
- Reject empty path after sanitization (400 Bad Request)
- Applied to: `path` query param on all endpoints, `filename` in Content-Disposition headers

Prevents XSS via filenames rendered in the frontend (no `<script>`, no unicode tricks, no null bytes).

### Layer 2: Filesystem traversal check (`resolve_workspace_path`)
- Looks up `session_dir` from `InstanceInfo` (running) or `PersistentRegistry` (stopped)
- Computes workspace root: `capsem_core::guest_share_dir(&session_dir).join("workspace")`
- Joins sanitized path, calls `canonicalize()` (resolves symlinks), verifies `starts_with` workspace root
- Returns 403 Forbidden on traversal (belt-and-suspenders with the sanitizer)

### Frontend defense in depth
- `sanitizePath()` utility in api.ts -- same `[a-zA-Z0-9._\-/]` allowlist before sending any path
- Svelte default text rendering (no `{@html}` for filenames)
- Dropped filenames sanitized before upload

## New Backend Endpoints

### `GET /files/{id}?path=&depth=2` -- List directory

Read `session_dir/guest/workspace/` with `std::fs::read_dir` + `metadata()` recursively.

Response:
```json
{
  "entries": [
    { "name": "src", "path": "src", "type": "directory", "size": 0, "mtime": 1713200000 },
    { "name": "main.rs", "path": "src/main.rs", "type": "file", "size": 1234, "mtime": 1713199000, "mime": "text/x-rust", "label": "rust", "is_text": true }
  ]
}
```

- Default depth=1, max depth=6
- Directories first, then alphabetical
- Skip hidden files (dot-prefixed), filter out `../system/` dir
- Magika detection on files at depth 1 only (read first ~8KB)

### `GET /files/{id}/content?path=src/main.rs` -- Download/read file

Returns raw bytes (not JSON-wrapped). Headers:
- `Content-Type`: Magika-detected MIME type
- `Content-Disposition: attachment; filename="main.rs"` (sanitized filename)
- `Content-Length`: from file metadata
- Reject files >10MB (413 Payload Too Large)

### `POST /files/{id}/content?path=src/main.rs` -- Upload/write file

Accepts raw bytes as request body (`Content-Type: application/octet-stream`).
- Writes to host-side workspace path
- Creates parent directories (`create_dir_all`)
- Mode 0o644
- 10MB max (gateway proxy enforces this)
- Response: `{ "success": true, "size": 1234 }`

## Frontend Changes

### API + types

New functions in `api.ts`:
- `listFiles(id, path?, depth?)` -> `FileListResponse` (JSON)
- `getFileContent(id, path)` -> `{ text: string; blob: Blob; size: number }` (fetches as blob, derives text)
- `uploadFile(id, path, content: Blob | string)` -> `{ success: boolean; size: number }`

New types in `types.ts`:
- `FileEntry` (name, path, type, size, mtime, mime?, label?, is_text?, children?)
- `FileListResponse` (entries: FileEntry[])

### FilesView.svelte -- replace find, add drag-and-drop, add refresh

- Replace `onMount` exec of `find /workspace` with `api.listFiles(vmId, '/', 4)`
- Remove `buildTreeFromPaths()` (response is pre-structured as tree)
- Add refresh button in header (ArrowClockwise icon from phosphor-svelte)
- Add error state with user-visible feedback
- Show file sizes in tree from API metadata
- Drag-and-drop: `dragenter`/`dragover`/`dragleave`/`drop` handlers on root div
- Visual overlay during drag: dashed border + "Drop files to upload" text
- On drop: `FileReader` + `api.uploadFile()` for each file, refresh tree after

### FileContent.svelte -- copy + download buttons, binary handling

- Add Copy button (CopySimple icon) in breadcrumb bar: `navigator.clipboard.writeText(content)`
- Add Download button (DownloadSimple icon): `URL.createObjectURL(blob)` + temp `<a download>` click
- Binary files: if `!is_text` from list response, show "Binary file (X KB) -- click to download" instead of syntax highlighting
- Use Magika `label` to improve Shiki language detection

## Key Files

### Backend
| File | Line(s) | What to change |
|------|---------|----------------|
| `crates/capsem-service/Cargo.toml` | deps | Add `magika = "1.0.1"`, add `tokio-util` with `io` feature |
| `crates/capsem-service/src/api.rs` | after L208 | Add `FileListEntry`, `FileListResponse`, `UploadResponse` structs |
| `crates/capsem-service/src/main.rs` | new fns | Add `sanitize_file_path()`, `resolve_workspace_path()` |
| `crates/capsem-service/src/main.rs` | new fns | Add `handle_list_files()`, `handle_download_file()`, `handle_upload_file()` |
| `crates/capsem-service/src/main.rs` | ~L2337 | Add routes: `/files/{id}`, `/files/{id}/content` |
| `crates/capsem-service/src/main.rs` | startup | Init shared `Magika` instance in `ServiceState` |

### Frontend
| File | What to change |
|------|----------------|
| `frontend/src/lib/api.ts` | Add `sanitizePath()`, `listFiles()`, `getFileContent()`, `uploadFile()` |
| `frontend/src/lib/types.ts` | Add `FileEntry`, `FileListResponse` (extend/replace `FileNode`) |
| `frontend/src/lib/components/views/FilesView.svelte` | Replace find-based tree, add drag-and-drop, refresh, error state |
| `frontend/src/lib/components/views/FileContent.svelte` | Add copy + download buttons, binary file handling |
| `frontend/src/lib/components/views/FileTree.svelte` | Show file sizes, use Magika label for icons |

### Existing code to reuse
| What | Where |
|------|-------|
| `capsem_core::guest_share_dir(session_dir)` | `crates/capsem-core/src/lib.rs:97` -- returns `session_dir/guest/` |
| `InstanceInfo.session_dir` | `crates/capsem-service/src/main.rs:69` -- per-VM session directory |
| `PersistentRegistry` | `crates/capsem-service/src/main.rs` -- stopped persistent VMs |
| `resolveShikiTheme()`, `detectShikiLang()` | `frontend/src/lib/shiki.ts` -- Shiki integration |
| `themeStore` | `frontend/src/lib/stores/theme.svelte.ts` -- theme state |
| Phosphor icons | Already imported: `FolderSimple`, `ArrowClockwise`, `DownloadSimple` in other components |
| Gateway proxy | `crates/capsem-gateway/src/proxy.rs` -- catch-all, no changes needed |

## Gateway Proxy -- No Changes Needed

The gateway proxy (`proxy.rs`) is a generic catch-all forwarding all requests to capsem-service over UDS. New endpoints are automatically proxied. The 10MB body limit (`MAX_BODY_SIZE`) applies to uploads. Response streaming for downloads works as-is.

## Verification

1. Boot a VM (`just shell`), open gateway UI, navigate to Files tab -- tree loads with real sizes and Magika labels
2. Click a text file -- content displays with syntax highlighting, copy + download buttons visible
3. Click copy -- content in clipboard (verify with paste)
4. Click download -- browser saves the file with correct name
5. Drag a text file from Finder onto Files pane -- uploads, appears in refreshed tree
6. Upload a binary file (PNG), download it, `diff` shows identical content
7. Upload a 5MB file -- works. 15MB -- rejected by gateway
8. Path traversal: `path=../../etc/passwd` -- returns 400 (sanitizer rejects `..`)
9. XSS filename: `path=<script>alert(1)</script>.txt` -- sanitized to `scriptalert1script.txt`
10. Null bytes / unicode tricks: `path=foo%00bar` -- stripped by allowlist
11. `cargo test -p capsem-service` passes
12. `cd frontend && pnpm test` passes
13. `cd frontend && pnpm run check` passes (zero warnings)
