# Mini-Capsem File UI Plugin Sprint

## Goal

Build a realistic isolated prototype of a Capsem plugin that reacts to a
`FileCreate` operation object, reads git metadata through an ABI-backed
filesystem read, fetches GitHub repository statistics through a Chrome-style
`fetch` intrinsic, returns the `FileCreate` object, and emits a UI block that a
gateway/remote UI could insert.

This is not a direct Capsem integration. It is a mini-Capsem harness inside the
independent Rust plugin prototype, shaped by the real Capsem architecture:

- File events come from host-side VirtioFS monitoring and session telemetry.
- `.git` is excluded from raw monitoring, so git inspection must be an explicit
  filesystem read capability.
- The UI cannot be directly mutated by WASM. Plugins emit typed UI blocks into a
  mutation channel that the host/gateway can route.
- Network access is not ambient. Plugins call `context.fetch(url)`, and the host
  enforces declared capability plus host policy before doing the request.

## Authoring Contract

The user-facing authoring shape is fluent and plugin-owned:

```ts
const plugin = Plugin("capsem.git-context")
  .on_file_create((file_event, context) => {
    if (file_event.path == ".git" || file_event.path.endsWith("/.git")) {
      const head = context.fs.read(file_event.path + "/HEAD");
      const config = context.fs.read(file_event.path + "/config");
      const github_api_url = github_repo_api_url(config);
      const github_stats = github_api_url.length > 0
        ? parse_github_stats(context.fetch(github_api_url))
        : "github stats unavailable";

      context.ui.sidePanel("workspace.context").replace({
        scope: { workspaceId: context.workspace.id },
        title: "Git Context",
        blocks: [
          new UiBlock("git-context-card", "git-context", "Git", head, github_stats),
        ],
      });
    }
    return file_event;
  });

export function on_file_create(
  objectPtr: i32,
  objectLen: i32,
  contextPtr: i32,
  contextLen: i32,
): i32 {
  return plugin.dispatch_file_create(objectPtr, objectLen, contextPtr, contextLen);
}
```

Callbacks return the mutated object. Filesystem reads, `fetch`, and UI block
emission are ABI calls.

## Files

- `crates/capsem-plugin-engine/src/lib.rs`
  - Add fs read, fetch, and UI block ABI imports to the Wasmtime executor.
  - Route ABI state from the copied context into the call state.
  - Attach emitted UI blocks to the returned object for the prototype harness.
- `crates/capsem-plugin-engine/tests/mini_capsem_file_ui.rs`
  - Compile the AssemblyScript plugin with `asc`.
  - Install it into the Rust engine.
  - Run a `FileCreate` object with fs context.
  - Assert returned git facts and emitted UI block.
- `examples/plugins/git_context_badge.ts`
  - Reviewable AssemblyScript authoring source for the plugin syntax.
- `docs/overview.md`
  - Note the fs/UI ABI pattern.

## Done Means

- The plugin source uses `Plugin("name").on_file_create(...)`.
- The callback returns the `FileCreate` object.
- The plugin reads `.git/HEAD` through `context.fs.read(...)`.
- The plugin reads `.git/config`, derives a GitHub API URL, and calls
  `context.fetch(...)`.
- The plugin updates a named UI surface through
  `context.ui.sidePanel(...).replace(...)`.
- The Rust host enforces capability-gated fs, fetch, and UI ABI calls.
- A Rust acceptance test compiles the plugin, runs it, and asserts:
  - git branch fact is present,
  - git project/worktree facts are present,
  - GitHub statistics are present from deterministic fetch fixtures,
  - UI mutation block is captured,
  - missing capability behavior is rejected or denied.

## Proof Matrix

- Unit/contract: host ABI path/URL validation and capability gating.
- Functional: acceptance test compiles and runs the real plugin.
- Adversarial: missing fs/fetch capability and unsafe path/URL attempts.
- Integration: mini-Capsem harness exercises registry install/run and Wasmtime.
- Telemetry/observability: emitted UI blocks are visible in returned trace result.
- Network: deterministic fixture-backed `fetch` test plus ignored live GitHub
  proof.
- Performance: deferred; this spike is product-shape proof.
