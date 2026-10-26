version: 0.7.2
---
Capsem 0.7.2 brings the single VM runtime and OCI application model to the
stable channel, with typed Python, TypeScript, and Rust SDKs, the npm host MCP
server, and the Inspect AI sandbox adapter.

### Sessions and applications

- Run applications from the OCI image catalog on one shared VM runtime.
- Browse workload Home files through the observed session file tree.
- Keep terminal sessions separate from graphical and web applications.
- Improve graphical application viewport sizing and Retina display behavior.
- Warm pause and checkpoint restore remain disabled while workspace handles
  cannot survive VM-owner replacement (#378). Persistent stop/start preserves
  files and restarts programs.

### Model inspection

- Group model reasoning, text, and matched tool results in Stats, with raw
  payloads available in Details.
- Preserve wrapped Google request model identity and price reviewed Gemini
  aliases through canonical rates.
- Account for Anthropic one-hour cache writes and mark truncated request
  excerpts accurately.
- Queue concurrent model and MCP inspection within bounded capacity.

### Release and verification

- Publish native binaries and VM runtimes independently, with immutable
  manifests and exact-package installation checks.
- Qualify packaged SDKs, the host MCP server, and Inspect against the native
  service before registry publication.
