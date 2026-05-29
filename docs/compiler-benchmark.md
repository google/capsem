# Plugin Compiler Benchmark

Recorded on 2026-05-21 in the isolated plugin-engine prototype.

## Question

Is the production compiler candidate fast enough for install-time plugin
compilation?

## Benchmark

Production-shaped AssemblyScript cases only. Reference lanes such as
`componentize-js` are intentionally excluded from this table because they are
not runnable through the current Rust core-module ABI.

Each row is compiled and smoke-run 5 times through the Rust engine. Time values
are rounded milliseconds except `Smoke run ms`, which is rounded to two
decimals because it is sub-millisecond.

| Case | Callbacks | Source bytes | Artifact bytes | Compile avg ms | Compile p50 ms | Compile min/max ms | Load avg ms | Smoke run ms | Verdict |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Minimal model output | `1` | `505` | `1,293` | `389` | `394` | `378 / 396` | `2` | `0.11` | Baseline production core-module shape. |
| Two callbacks + shared helpers | `2` | `1,004` | `1,579` | `386` | `385` | `382 / 393` | `1` | `0.09` | Multiple exported hooks do not move compile cost. |
| SDK-ish security plugin | `4` | `2,240` | `2,844` | `414` | `411` | `406 / 426` | `1` | `0.21` | Representative generated-SDK shape is still cheap enough. |

## Conclusion

AssemblyScript remains viable for the next production spike. The meaningful
case is the SDK-ish security plugin: 4 callbacks, decisions, findings, and
patch output compiled in about `414ms`, emitted a `2.8KB` artifact, loaded in
about `1ms`, and smoke-ran through the Rust WASM ABI in about `0.21ms`.

The next compiler benchmark should use the real generated SDK helpers and
schema bindings. The current measurement says the compiler path itself is not
the scary part.

## Command

```bash
cargo run -p capsem-plugin-engine --release --example compiler_spike
```
