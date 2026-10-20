# SDK reference generation

Package READMEs are handwritten. The include-only symlinks in
`src/content/sdk-readmes/` render them in the shared SDK tutorial.

The three reference pages are generated from current source:
Python AST declarations and docstrings, compiler-produced TypeScript declarations,
and compiler-produced Rust declarations including the shared capsem-api types.

Docs CI regenerates Python, TypeScript, and the host MCP tool catalogue before
building the site. SDK/MCP source changes trigger that workflow too. Pull requests
reject stale checked-in references; pushes to `main` deploy freshly generated
content. Rust still uses the explicit rustdoc command below.

To refresh the CI-generated pages locally, run these commands through the bounded wrapper:

```sh
pnpm --dir mcp/typescript install --frozen-lockfile
pnpm --dir sdk/typescript run build
pnpm --dir mcp/typescript run build
uv run --project build_system --frozen python web/docs/tools/generate-sdk-reference.py --languages python typescript
node web/docs/tools/mcp-reference.mjs
```

The MCP generator connects to the real server over an in-memory MCP transport
and calls only `tools/list`, including pagination. It never invokes a tool or
contacts a gateway. Only the marked catalogue block is generated; connection,
permissions, guest tools, and recovery instructions remain handwritten.
Declared input/output schemas and annotations come directly from the listing.
Add `--check` to either generator to detect drift without writing.

From the repository root, run each command through the bounded wrapper:

```sh
uv run --project build_system --frozen python build_system/scripts/ci/run-bounded-command.py --timeout-seconds 90 -- pnpm --dir sdk/typescript run build
uv run --project build_system --frozen python build_system/scripts/ci/run-bounded-command.py --timeout-seconds 300 -- cargo doc -p capsem-sdk -p capsem-api --no-deps
uv run --project build_system --frozen python build_system/scripts/ci/run-bounded-command.py --timeout-seconds 60 -- uv run --project build_system --frozen python web/docs/tools/generate-sdk-reference.py --rustdoc-dir /Users/elie/git/capsem/cache/target/cargo/doc
```

Use the cargo doc output path reported by Cargo if the checkout uses another
shared target root. Regenerate declarations and rustdoc on the same source
revision before generating Markdown. Append `--check` to the final command
to reject drift without writing.

The public onboarding examples use implemented local gateway discovery and
omit URL/token setup. Check registry availability separately from source API
compatibility: npm currently serves 0.7.0, while the 0.7.1 Python and Rust
instructions pin source until their packages are published. Regenerating a
reference does not prove that a VM example ran successfully.
