# SDK reference generation

Package READMEs are handwritten. The include-only symlinks in
`src/content/sdk-readmes/` render them in the shared SDK tutorial.

The three reference pages are generated from current source:
Python AST declarations and docstrings, compiler-produced TypeScript declarations,
and compiler-produced Rust declarations including the shared capsem-api types.

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
