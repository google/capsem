# Sprint: JS to WASM Component Spike

## Goal

Prove the production-shaped plugin execution lane:

```text
JavaScript module
  -> fixed WIT callback ABI
  -> componentize-js / jco
  -> WebAssembly component
  -> execute callback
  -> returned object copy
  -> timings
```

This sprint intentionally avoids alternate runtimes and partial proofs. We are
not testing "can JavaScript run somewhere." We are testing whether the future
Capsem plugin ABI can be authored as JavaScript, compiled to a WASM component,
executed through the component boundary, and return an object result with
acceptable performance.

## Non-Goals

- No Javy lane.
- No in-process-only runtime shortcut.
- No Capsem repo integration.
- No plugin marketplace/install flow.
- No full security object algebra.
- No real host ABI calls for HTTPS/model access.
- No LSP work.

Those become future sprints only if this lane works.

## User-Facing Spike API

Provide a local HTTP service:

```http
POST /
```

Request:

```json
{
  "source": "export const plugin = { invoke(objectJson, contextJson) { return objectJson; } }",
  "object": {
    "kind": "ModelOutput",
    "content": { "text": "hi" }
  },
  "context": {
    "trace": { "labels": [] }
  },
  "functions": ["invoke", "inspect"],
  "runs": 3
}
```

Response on success:

```json
{
  "ok": true,
  "compile_ms": 412,
  "instantiate_ms": 9,
  "run_ms": 2,
  "wasm_bytes": 10485760,
  "result": {
    "kind": "ModelOutput",
    "content": { "text": "hi" }
  },
  "traces": [
    {
      "function": "invoke",
      "iteration": 1,
      "run_ms": 2,
      "result": {
        "kind": "ModelOutput",
        "content": { "text": "hi" }
      },
      "result_bytes": 52
    }
  ],
  "functions": ["invoke"],
  "runs": 1
}
```

Response on failure:

```json
{
  "ok": false,
  "phase": "compile",
  "error": "..."
}
```

## Fixed ABI

Use one fixed WIT world for the spike:

```wit
package capsem:plugin-spike;

world plugin {
  export invoke: func(object-json: string, context-json: string) -> string;
  export inspect: func(object-json: string, context-json: string) -> string;
}
```

The JavaScript module must satisfy this shape:

```js
export function invoke(objectJson, contextJson) {
  const object = JSON.parse(objectJson);
  const context = JSON.parse(contextJson);

  object.seen = context.trace?.labels ?? [];
  return JSON.stringify(object);
}

export function inspect(objectJson, contextJson) {
  const object = JSON.parse(objectJson);
  return JSON.stringify({
    kind: "Inspection",
    object_kind: object.kind,
    object_keys: Object.keys(object)
  });
}
```

If `jco`/`componentize-js` requires a different exact export shape for this
WIT, adapt the wrapper code but keep the public HTTP request shape stable.

## Architecture

```text
HTTP service
  -> create temp isolated build dir
  -> write source module
  -> write fixed WIT
  -> run jco/componentize-js
  -> instantiate/execute component
  -> parse returned JSON
  -> return timings/result
  -> cleanup temp dir
```

The service must collect:

- `compile_ms`
- `instantiate_ms`
- `run_ms`
- `wasm_bytes`
- per-callback traces with callback name, iteration, run time, result, and result size
- phase-specific errors

The service must enforce:

- request body size limit
- source size limit
- object/context serialized size limits
- compile timeout
- run timeout
- output size limit
- per-request temp directory
- cleanup after success/failure

## Execution Choice

Use the component model toolchain closest to production:

- `@bytecodealliance/jco`
- `@bytecodealliance/componentize-js` through `jco componentize`

The execution path may use whichever component invocation method is most direct
for the spike:

- `jco transpile` and Node import of the generated wrapper, or
- Wasmtime/component invocation if readily available in the local toolchain.

Do not add a second JS-to-WASM lane.

## Files To Create

Expected shape:

```text
package.json
tsconfig.json
src/server.ts
src/component-runner.ts
src/types.ts
src/limits.ts
src/timing.ts
test/compile-run.test.ts
fixtures/
```

If the implementation language changes during discovery, update this plan
before coding further.

## Testing Matrix

Unit/contract:

- request validation rejects oversized source/object/context
- response shape is stable on success
- response shape is stable on compile/run failure
- timing fields are present and numeric

Functional:

- simple callback returns input object unchanged
- callback reads context and modifies returned object
- callback returns structured object with mutation-like fields
- two callbacks compile into the same WASM component and can be invoked repeatedly after one import

Adversarial:

- syntax error reports `phase: "compile"`
- callback throws reports `phase: "run"`
- callback returns invalid JSON reports `phase: "result"`
- infinite loop or long-running callback times out
- huge returned string is rejected

Performance:

- benchmark loop records compile/transpile/instantiate/run timings across repeated callback calls
- report artifact size

Telemetry/logging:

- service logs one structured record per request with phase, timings, and success/failure

E2E/VM:

- deferred; this sprint is intentionally independent of Capsem.

## Done

This sprint is done when:

- `POST /` compiles JavaScript to a WASM component through the fixed WIT ABI.
- The service executes the component callback with object/context JSON.
- The service returns the parsed object result, per-call traces, and timing fields.
- Tests prove success, compile failure, run failure, invalid result, and timeout behavior.
- A benchmark command or test prints enough timing data to decide whether the lane is viable.

If the component toolchain cannot support this callback shape, stop and record
the blocker with exact command output and the closest working shape.
