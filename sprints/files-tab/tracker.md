# Sprint: Files Tab

## Status: In Progress

## T1: Backend -- Path Security + Magika Init

- [x] Add `magika` to `crates/capsem-service/Cargo.toml`
- [x] Add `tokio-util = { version = "0.7", features = ["io"] }` to capsem-service deps
- [x] Add `sanitize_file_path(path: &str) -> Result<String>` -- allowlist `[a-zA-Z0-9._\-/]`, reject `..`, collapse slashes
- [x] Add `resolve_workspace_path(state, id, sanitized_path) -> Result<(PathBuf, PathBuf)>` -- lookup session_dir, canonicalize, starts_with check
- [x] Add `Mutex<magika::Session>` to `ServiceState` -- init via `Session::new()` or `Session::builder().with_inter_threads(1).build()`
- [x] Add helper `fn identify_file(magika: &Mutex<Session>, path: &Path) -> (String, String, String, bool)` returning (label, mime_type, group, is_text) from `TypeInfo`
- [x] Unit tests: sanitize_file_path strips `<script>`, null bytes, unicode, `..` traversal
- [x] Unit tests: resolve_workspace_path rejects symlink escape, path outside workspace
- [x] cargo test -p capsem-service passes
- [x] Commit: `feat: path sanitization and Magika init for files API`

## T2: Backend -- List Directory Endpoint

- [ ] Add `FileListEntry` struct to api.rs (name, path, type, size, mtime, mime, label, is_text)
- [ ] Add `FileListResponse` struct to api.rs (entries: Vec<FileListEntry>)
- [ ] Implement `handle_list_files(state, id, query_params)` handler
- [ ] Recursive `read_dir` + `metadata()` up to max depth (default 1, max 6)
- [ ] Magika detection on files at depth 1 only (first ~8KB)
- [ ] Skip hidden files (dot-prefix), filter out `system/` directory
- [ ] Sort: directories first, then alphabetical
- [ ] Register route: `.route("/files/{id}", get(handle_list_files))` at ~L2337
- [ ] Test: list returns correct tree structure
- [ ] Test: respects depth limit
- [ ] Test: returns 404 for nonexistent VM
- [ ] Test: path traversal blocked (400/403)
- [ ] cargo test -p capsem-service passes
- [ ] Commit: `feat: GET /files/{id} directory listing endpoint`

## T3: Backend -- Download + Upload Endpoints

- [ ] Implement `handle_download_file(state, id, query_params)` handler
- [ ] Return raw bytes with Content-Type from Magika, Content-Disposition with sanitized filename
- [ ] Reject files >10MB (413)
- [ ] Implement `handle_upload_file(state, id, query_params, body)` handler
- [ ] Accept raw bytes, write to workspace path, create_dir_all for parents, mode 0o644
- [ ] Return JSON `{ success: true, size: N }`
- [ ] Register routes: `.route("/files/{id}/content", get(handle_download_file).post(handle_upload_file))`
- [ ] Test: download returns correct bytes and headers
- [ ] Test: download binary file preserves content
- [ ] Test: upload creates file with correct content
- [ ] Test: upload creates parent directories
- [ ] Test: upload path traversal blocked
- [ ] Test: download 404 for nonexistent file
- [ ] cargo test -p capsem-service passes
- [ ] Commit: `feat: file download and upload endpoints via host-side VirtioFS`

## T4: Frontend -- API Functions + Types

- [ ] Add `sanitizePath(path: string): string` to api.ts -- `[a-zA-Z0-9._\-/]` allowlist
- [ ] Add `listFiles(id, path?, depth?)` to api.ts -- calls `GET /files/{id}`
- [ ] Add `getFileContent(id, path)` to api.ts -- fetches as blob, returns `{ text, blob, size }`
- [ ] Add `uploadFile(id, path, content: Blob | string)` to api.ts -- POST raw bytes
- [ ] Add `FileEntry` interface to types.ts (name, path, type, size, mtime, mime?, label?, is_text?, children?)
- [ ] Add `FileListResponse` interface to types.ts
- [ ] Update imports in api.ts (add new types to import block)
- [ ] pnpm run check passes
- [ ] Commit: `feat: frontend API client for files endpoints`

## T5: Frontend -- Replace Find-Based Tree

- [ ] Replace `onMount` in FilesView.svelte: call `api.listFiles(vmId, '/', 4)` instead of `execCommand('find ...')`
- [ ] Remove `buildTreeFromPaths()` function (response is pre-structured)
- [ ] Map `FileEntry` to existing `FileNode` shape or update FileTree to accept FileEntry directly
- [ ] Add refresh button in header bar (ArrowClockwise icon from phosphor-svelte)
- [ ] Add error state: show message when listing fails
- [ ] Add loading state with spinner/skeleton
- [ ] Update FileTree.svelte to show file sizes next to names
- [ ] Update file selection to use `getFileContent()` instead of `readFile()`
- [ ] pnpm run check passes
- [ ] Visual test: Files tab loads tree with real sizes for a running VM
- [ ] Commit: `feat: replace find-based file tree with host-side listing`

## T6: Frontend -- Copy + Download Buttons

- [ ] Add Copy button (CopySimple icon) to FileContent.svelte breadcrumb bar
- [ ] Implement copy: `navigator.clipboard.writeText(content)`
- [ ] Add brief "Copied!" feedback (tooltip or text flash)
- [ ] Add Download button (DownloadSimple icon) to FileContent.svelte breadcrumb bar
- [ ] Implement download: `URL.createObjectURL(blob)` + temp `<a download="filename">` click
- [ ] Handle binary files: if `!is_text`, show "Binary file (X KB) -- click to download" instead of Shiki
- [ ] Use Magika `label` field to improve Shiki language detection where available
- [ ] pnpm run check passes
- [ ] Visual test: buttons appear, copy works, download saves correct file
- [ ] Commit: `feat: copy and download buttons in file viewer`

## T7: Frontend -- Drag-and-Drop Upload

- [ ] Add `dragenter`/`dragover`/`dragleave`/`drop` handlers to FilesView.svelte root div
- [ ] Add `dragActive` state for visual overlay
- [ ] Style overlay: dashed border, primary color, "Drop files to upload" text (Preline CSS classes)
- [ ] On drop: extract files from DataTransfer
- [ ] Sanitize dropped filenames with `sanitizePath()`
- [ ] Determine target: selected directory path or workspace root
- [ ] Call `api.uploadFile(vmId, targetPath, file)` for each dropped file
- [ ] Show upload progress/status (inline message or brief toast)
- [ ] Refresh tree after successful upload
- [ ] Handle errors (file too large, upload failed) with user-visible message
- [ ] pnpm run check passes
- [ ] Visual test: drag file from Finder, overlay appears, file uploads and shows in tree
- [ ] Commit: `feat: drag-and-drop file upload in Files tab`

## T8: Integration Testing + Polish

- [ ] Boot VM, open Files tab -- tree loads with metadata
- [ ] Click file -- syntax highlighted with copy + download buttons
- [ ] Copy button -- content in clipboard
- [ ] Download button -- browser saves file
- [ ] Drag-and-drop text file -- uploads and appears in tree
- [ ] Drag-and-drop binary file (PNG) -- uploads, downloads identical
- [ ] Path traversal blocked (test via browser dev tools)
- [ ] XSS filename sanitized (test via browser dev tools)
- [ ] `cargo test -p capsem-service` passes
- [ ] `cd frontend && pnpm test` passes
- [ ] `cd frontend && pnpm run check` passes (zero warnings)
- [ ] Commit: `chore: files tab integration tests and polish`

## Notes

- Gateway proxy (capsem-gateway/src/proxy.rs) needs zero changes -- catch-all forwarding handles new routes
- Gateway 10MB body limit (`MAX_BODY_SIZE`) is the upload ceiling
- Existing `read_file`/`write_file` vsock endpoints remain for MCP tools (capsem_read_file, capsem_write_file)
- Preline JS components (HSTreeView, HSFileUpload, HSCopyMarkup) were evaluated and rejected -- using pure Svelte + Preline CSS instead
- SCP is planned separately for large file transfers beyond the 10MB API limit

### Magika integration notes

- Crate: `magika` on crates.io. Embeds ONNX model via `include_bytes!` -- no external model files
- `magika::Session::new()` loads the model. Store as `Mutex<Session>` in `ServiceState` (identify methods take `&mut self`)
- `session.identify_file_sync(path)` -> `FileType` (reads file internally)
- `session.identify_content_sync(bytes)` -> `FileType` (from in-memory buffer)
- `FileType::Inferred(inferred)` -> `inferred.content_type.info()` -> `TypeInfo { label, mime_type, group, description, extensions, is_text }`
- `FileType::Ruled(ct)` -> `ct.info()` -> same `TypeInfo`
- Score: `inferred.score` (f32, 0.0-1.0)
- 217+ file types, covers code/docs/images/archives/video/audio
- Async variants available: `identify_file_async`, `identify_content_async`
- Builder: `Session::builder().with_inter_threads(1).with_intra_threads(1).build()` for thread control
