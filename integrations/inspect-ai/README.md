# Inspect AI integration (`inspect-capsem-sandbox`)

`inspect-capsem-sandbox` (`inspect_capsem`) registers the `capsem` [`SandboxEnvironment`](https://inspect.aisi.org.uk/sandboxing.html) for [Inspect AI](https://inspect.aisi.org.uk/), running evaluations inside isolated Capsem micro-VMs via the Python [`capsem`](../../sdk/python/README.md) SDK. It is carried from Pierre Tholoniat's original implementation in [#291](https://github.com/google/capsem/pull/291) (commit `bb61fc82d44bc42c978c4acf5435a1975600c75e`) and is not published to PyPI yet.

From a repository checkout (or via `pip install inspect-capsem-sandbox` once published):

```bash
uv sync --project integrations/inspect-ai --frozen
uv run --project integrations/inspect-ai inspect eval task.py --sandbox capsem
```

See the [Inspect AI Sandbox documentation](https://docs.capsem.org/usage/inspect-ai/) (`web/docs/src/content/docs/usage/inspect-ai.md`) for configuration (`CapsemSandboxConfig`), environment variables, and lifecycle behavior.

The current source provides the original pure Dockerfile instruction scanner,
heredoc transformations, multistage CA patcher and final-stage USER/WORKDIR
metadata extraction. `inspect_capsem.containers.dockerfile` remains their
canonical import; the implementations are split into small owning modules.
The transformations accept text and have no host discovery, credential,
filesystem, build-engine or session-lifecycle authority.

Compose parsing preserves the proposed single-service fields, interpolation,
project dotenv semantics, build arguments/targets, mounts, ports, healthcheck,
init, memory and network intent. `ComposeInputs` is an immutable snapshot of
environment, file text and directory facts supplied by the evaluator. File
parsing reads only that snapshot; absent text is refused. These values confer
no permission to read host paths or access credentials. The controller must
obtain grants and validate regular files and transfer limits before creating
the snapshot.

The evaluator must supply `ComposeLimits` for bytes, nodes and depth. Limits
cover YAML construction, merge expansion, alias traversal, variable expansion
and intermediate fragments. Interpolation/YAML diagnostics avoid echoing
variable values or malformed dotenv lines.

Classic-frontend heredoc lowering is an optional transform. It does not select
a build engine or replace full Dockerfile execution. Full unification onto the
evaluator-controlled `ComposeInputs` snapshot, native builder acceptance, and
standalone wheel/sdist package qualification remain in progress under
[#310](https://github.com/google/capsem/issues/310) and
[#311](https://github.com/google/capsem/issues/311). Preserve the original
integration's complete behavior when carrying those components.

Focused source checks use the repository's frozen development environment:

```sh
python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 60 -- \
  env PYTHONPATH=integrations/inspect-ai uv run --project build_system --frozen \
  python -m pytest -c build_system/pyproject.toml --rootdir . \
  integrations/inspect-ai/tests/containers -q
```

These tests prove the carried text transformations, including actual bounded
shell/interpreter semantics. They do not qualify a native builder or Inspect
sandbox. Native acceptance and package publication require their full gates.
