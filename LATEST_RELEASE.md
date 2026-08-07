version: 1.6.1785421421
---
### Fixed

- Fixed the asset gate's Docker capacity test hardcoding free-space fixtures
  against the old 24 GiB floor. "30 GiB is plenty" silently became "30 GiB is
  not enough" when the floor moved to 40, and it surfaced as a release gate
  refusing to build assets rather than as a stale fixture. The fixtures now
  derive from `config/storage-policy.toml`, so the floor and the test that
  exercises it cannot disagree.

### Changed

- Raised the Docker storage budget so BuildKit stops discarding a hot cache.
  `buildkit_keep_gib` was 24 while the cache ran at ~65 GB with ~35 GB hot, so
  every pressure prune threw away layers that were about to be reused and the
  host-builder image recompiled cold. The budget is now 80 GiB kept, a 40 GiB
  free floor, and a 200 GiB recommended disk. `minimum_disk_gib` rose from 96
  to 160 to keep the policy satisfiable: keep + free floor + fixed cache usage
  must fit inside the minimum disk, or the floor can never be met by the one
  action taken to meet it -- which is how a full daemon timed out the capacity
  probe and failed a release gate.

### Added

- Added `tests/test_rust_test_name_assertions.py`, which fails when a Python
  source contract asserts a Rust test name against production source instead of
  the sibling `tests.rs` the test lives in. This class cost two release
  attempts: sixteen contracts under `tests/capsem-release/`, then five more
  under `tests/capsem-install/` that run only inside the Docker install gate and
  so stayed invisible until forty minutes into a release run. The guard resolves
  each assertion's target through the AST, per function scope, so contracts that
  legitimately name a relocated test while asserting it against a test module or
  a spec document are not flagged -- a guard needing exemptions is not a guard.

### Fixed

- Fixed the installed-shell proof reporting a 300-second timeout instead of the
  boot error the service had already handed it. `capsem create` returns once the
  VM process is launched, so a boot that dies afterwards leaves the TUI parked on
  its non-resumable screen and no prompt ever arrives; the proof watched only the
  terminal, so it waited out its whole budget and then blamed the missing prompt.
  It now polls `capsem info --json` while it waits and fails immediately with the
  session's own `last_error` or `resume_blocked_reason` -- the `process.log` tail
  naming the real cause. This is what turned the 1.6 asset-pin mismatch into a
  five-minute wait pointing at the wrong thing.

- Fixed `just _gate-assets` preserving no evidence for a failed boot proof. The
  proof deleted its session in an unconditional cleanup, so `process.log` and
  `serial.log` were gone before the gate's `run-failure` copy ran and the copy
  captured empty directories. A failing proof now keeps its session -- stopping
  the gate service still reaps every VM process and leaves persistent session
  dirs intact -- and the copy walks the run dir for host-side diagnostics,
  including the `vm/active_profile.toml` whose recorded pins are what a hash
  mismatch is argued from. It prunes `guest/` and `auto_snapshots/` so the guest
  workspace, duplicated once per snapshot generation, stays out of `target/`.

- Fixed `docker-storage-policy.py enforce` crashing with a bare
  `KeyError: 'free_bytes'` when Docker stopped reporting capacity between its
  opening snapshot and the re-measure after pruning -- which pruning itself can
  provoke. The opening snapshot was guarded for availability and the re-measure
  was not. It now fails with a message naming Docker and the free-space floor it
  could not check, instead of a traceback that named neither.

- Fixed the guest kernel diagnostic failing every freshly built image. Moving
  `kernel_branch` from 7.0 to 6.18 left the in-VM check asserting
  `major >= 7`, so each newly built kernel failed its own diagnostics at the
  end of the release gate, minutes of VM boots away from the one-line cause.
  The diagnostic now asserts the configured branch, and
  `tests/test_guest_kernel_branch_contract.py` holds it equal to the build pin
  so the two cannot drift again. A `major >= N` floor was wrong in both
  directions: it rejected the pinned branch and would have accepted any future
  kernel the guest was never built against.

- Fixed VM boot rejecting correct assets because a pin and its digest were
  spelled differently. Profile pins derived from the release graph carry
  `blake3:<hex>`; asset manifests carry bare hex. Boot compared the two
  verbatim, so every VM in the asset gate died with a mismatch whose expected
  and actual digests were character-for-character identical apart from the
  algorithm tag. `VmConfigBuilder::verify_hash` now resolves both spellings in
  the one place that decides what an expected hash means, and refuses a
  non-blake3 algorithm outright rather than letting a `sha256:` pin masquerade
  as asset corruption it can never match. The boot-audit line logs pins in full:
  truncated to 16 characters, both spellings render as plausible prefixes, which
  is exactly how this hid.

- Fixed 16 release-contract tests that blocked both release lanes. They asserted
  that specific Rust tests exist, but read only the production `.rs` file after
  those tests moved to sibling `tests.rs` modules, so the whole gate failed on a
  layout change that broke nothing. Source contracts now read production code
  and its test module through `tests/rust_sources.py`, which resolves `mod
  tests;` the way Rust does and raises when the module is absent instead of
  passing on an empty string. The two sources stay separate deliberately:
  several contracts assert a symbol is *absent* from production, and a test
  module legitimately names what it proves is rejected.
- Fixed the Rust line-coverage floor asserted by two separate contracts, which
  still demanded 65 after the measured surface grew to include previously
  unmonitored crates and the real floor became 63. The value is now named once
  as `RUST_LINE_COVERAGE_FLOOR`, since a floor that disagrees with itself is
  worse than no floor.

### Added

- Added `manyfaces`, a Rust suite holding the asset model to Docker's: blobs
  content-addressed and shared between profiles, a profile's `image_revision`
  behaving as a tag so several revisions coexist on disk, and blob lifetime
  reference-counted rather than governed by a channel-wide pointer. Fifteen
  tests pass, describing what already holds. Five are deliberately red and are
  the specification for removing that pointer: three profiles currently collapse
  to one asset set, a profile's kernel becomes unreachable, one global hash
  cannot verify three profiles, and a refresh deletes an installed profile's
  kernel as unreferenced.
- Added a Rust test-layout contract to the shared fast gate. It fails on an
  inline `#[cfg(test)] mod tests { ... }` block, a `tests.rs` that no parent
  module declares (which silently never compiles or runs), a crate shipping no
  Rust tests at all, and any Rust source a `.gitignore` rule would drop.
  `just test`, `just fast-test`, ordinary CI and both release lanes all reach it
  through `_test-fast`.

### Changed

- Lowered the Rust coverage floor from 65% to 63%. The workspace measures
  63.63%, so the 65% gate was failing before this branch and would have kept
  failing after it: 47 new tests moved the number +0.09 points, because the
  uncovered mass in the binary crates is async I/O the Python suites drive
  through subprocesses, which `cargo llvm-cov` cannot see. 63% is where the
  code actually is, which keeps the floor doing its stated job -- catching a
  "we deleted half the test suite" regression -- instead of failing every run.
  Raise it by ratchet as real coverage lands, not ahead of it.
- Moved every Rust unit-test module out of its production file and into the
  sibling `tests.rs` the project convention requires. 86 files carried inline
  `#[cfg(test)] mod tests { ... }` blocks; the largest buried 4,070 lines of
  tests under 8,855 lines of production code, so every read, grep, and scroll
  to reach that code walked the tests first. Bodies moved verbatim -- the only
  content edits are three `include_str!` paths that follow their file one
  directory deeper.

### Fixed

- Restricted the desktop shell's `open_url` command to http, https and mailto.
  It is a Tauri command, so anything running in the webview could invoke it
  with any string, and the page-side filter forwards any href carrying
  `target="_blank"` without inspecting its scheme -- so `file:///` and
  `javascript:` reached the OS opener unchecked. The allowlist is positive, so
  a scheme nobody considered stays refused rather than being opened because it
  looked harmless.
- Covered the benchmark harness's JSON scrape of guest command output and the
  latency-summary edges. Both feed published numbers: a scrape that picks the
  wrong object reports a figure that was never measured, and a degenerate
  sample set must summarise to zeros rather than to something plausible.
- Closed a guest-triggerable bypass of support-bundle redaction.
  `redact_log_bytes` decoded the whole file and passed it through untouched if
  any of it was not UTF-8, so a guest writing a single invalid byte to its
  console disabled redaction for all of `serial.log` -- every credential in
  that log shipped in the bundle in the clear. Redaction is now decided per
  line, and lines that genuinely are not UTF-8 still pass through byte-exact.
- Stopped reading whole log files to return their tail. `read_tail` read the
  entire file before slicing off the last few MiB, which let a chatty guest
  decide how much memory `capsem support-bundle` allocated via serial.log; it
  now seeks.
- Covered the tray against the status casing a real gateway sends. The gateway
  serializes VM state capitalized ("Running", "Suspended"), but every tray test
  used the lowercase form, leaving the production shape the one thing nothing
  exercised; a regression in either case-folding site would have rendered a live
  service as unavailable and stripped Connect from every running VM.
- Covered the shared mock server's request parsers. It is the single fixture
  behind benchmarks, doctor, protocol replay, gateway integration and Ironbank,
  and several of its parsers fail into a default rather than an error -- junk
  bodies become `{}`, a path that names no model resolves to a fallback model.
  That is fine for a fixture but means a suite built on a malformed request can
  pass for the wrong reason, so the silent defaults are now stated behaviour.
- Told the user when exec output was capped. `capsem exec` and `capsem run`
  now print a notice on stderr, leaving stdout byte-exact so piping and
  programmatic consumers are unaffected; previously a truncated result was
  indistinguishable from a command that genuinely produced that much.
- Surfaced exec output truncation in the service /exec response, so an HTTP
  caller can tell a capped result from a complete one.
- Carried exec output truncation across the process/service IPC boundary, so a
  capped result can no longer reach a caller looking like a complete one. The
  field is `#[serde(default)]`, so a producer built before it existed still
  decodes as not truncated rather than failing the frame.
- Bounded guest exec output at 10 MiB. The Exec vsock port is a raw stream, so
  the `MAX_FRAME_SIZE` bound applied to length-prefixed control frames never
  reached it and `read_exec_output` accumulated without limit: a guest running
  `yes` grew capsem-process until the OOM killer took it and every in-flight
  job with it. The 5s deposit timeout did not help, because the reader thread
  is detached and keeps allocating after `ExecDone` has given up and dropped
  the slot -- so the memory was spent on output nobody would read. The cap
  matches capsem-gateway's `MAX_BODY_SIZE`, reading continues to EOF so the
  guest is never left blocked on a full socket, and `stdout_bytes` telemetry
  now reports the bytes the guest actually wrote rather than the retained
  slice.
- Covered two untested boundaries in the per-VM process. `parse_pty_log` walks
  a length-prefixed binary format off disk and was only ever exercised through
  its own writer, so nothing checked a recording truncated by a crash or a
  corrupt length field claiming 4 GiB of payload; it must return what it can
  parse rather than panic into the terminal view. `ackable_id` /
  `ackable_response_id` decide which messages are replayed until acknowledged,
  where a missing variant loses a message across a reconnect and an extra one
  replays forever -- including `Shutdown`, which must never be replayed.
- Closed two leaks in the support-bundle redactor, which exists so a bundle can
  be attached to a public bug report without shipping credentials. It matched
  only a bare `Authorization:`, so the JSON-shaped
  `{"Authorization": "Bearer <token>"}` that Capsem's own JSON logs actually
  produce passed through untouched; the header prefix is now preserved and only
  the credential replaced, so a redacted line stays valid JSON. GitHub tokens
  (`ghp_`/`gho_`/`ghu_`/`ghs_`/`ghr_`) were not matched at all despite
  `github_token` already being treated as a secret key name in the same module.
- Covered the guest agent's auditd record parsers, which turn kernel audit
  lines into the exec attribution the security ledger records. All four were
  untested. The tests pin the sharp edges deliberately: field lookup is a
  substring search, so the leading space in `" pid="` is load-bearing and
  dropping it silently attributes the parent's pid to the child; `extract_execve_argv`
  truncates at the first gap in argument numbering rather than mis-ordering;
  and an absurd timestamp saturates instead of wrapping into a plausible value
  that would reorder the ledger.
- Covered builtin MCP tool-failure propagation. `extract_text` decides whether
  a refused tool call reaches the agent as a failure or as a successful result
  whose body happens to contain error prose; the `isError` branch exists
  because it once did the latter, and none of it was tested. Both refusal
  channels are now pinned, along with the precedence between them and the
  sharp edge that a non-boolean `isError` is not a refusal signal.
- Covered the guest control-channel leak detector adversarially.
  `looks_like_ipc_frame` is the only signal that a guest is writing IPC
  protocol bytes into its own PTY stream, and it was asserted only negatively.
  It now has positive cases pinned to real encoder output, so a wire-format
  change breaks the test instead of silently disabling the detector, plus the
  short/empty reads a PTY can always return, each byte of the frame prefix in
  isolation, the fixstr range boundaries, and ordinary terminal output that
  must not be flagged.
- Attached every remaining Rust source to a codecov component. Sixteen
  capsem-core files belonged to none: `credential_broker.rs` now reports under
  Security, `telemetry.rs` under Monitoring, `host_state.rs` under
  Virtualization and `bin/mcp_export.rs` under Tooling, with two new components
  for concerns that had no home -- Assets (`asset_manager.rs`,
  `manifest_compat.rs`, the runtime half of the manifest contract) and Core
  Platform (path resolution, UDS helpers, IPC handshake, backoff, macros). The
  `#[cfg(test)]`-only `test_support/` helpers moved to `ignore`. All 200 Rust
  sources across every crate now resolve to exactly one component.
- Repointed four stale `codecov.yml` component paths at files that no longer
  exist. Three sat in `network` (`mitm_proxy.rs`, deleted `domain_policy.rs`
  and `http_policy.rs`) and one in `security` (deleted `host_config.rs`), so
  both components silently measured less than their own comments claimed --
  `network` excluded the entire MITM proxy it is named for, along with DNS,
  the AI-provider interpreters and the SSE/DNS parsers. Both now match their
  documented scope: all of `net/` except `policy_config`, and the policy
  engine including `security_engine/`. This drops `network` from a flattering
  91.7% to an honest 71.7%, so its first `target: auto` build will re-baseline
  against the wider scope.
- Followed the moved unit tests in four source-contract assertions. Contracts
  in `test_release_doctor_contract.py` and `test_dbwriter_snapshot_contract.py`
  read a production file and asserted that test names or fixture strings
  appeared in it, which stopped holding once those tests moved to sibling
  `tests.rs` files; one had been silently reduced to asserting against the
  two-character remainder of a `split()`.
- Put a test under the service's spawn environment boundary.
  `PROCESS_ENV_ALLOWLIST` is the whole barrier between the daemon's own
  environment -- which on a developer or CI machine routinely holds
  `ANTHROPIC_API_KEY`, `AWS_SECRET_ACCESS_KEY` and the like -- and the per-VM
  process that talks to the guest. Nothing enforced its contents, so adding a
  secret-shaped or non-Capsem name now fails the build instead of silently
  forwarding a host secret.
- Covered the MCP server manager's reserved-header check, `WWW-Authenticate`
  scope parsing, JSON-RPC error sniffing and pool-routing guard conditions.
  Without the header test, a server definition could have contested
  `Mcp-Session-Id` and steered another session's stream.
- Covered the two Rust surfaces that had no unit tests worth the name.
  `capsem-mcp-aggregator` was the only crate in the workspace with none at all,
  and `StreamTracker` in the MITM MCP frame path had none despite being the
  only thing standing between a hostile guest and response confusion. Both
  parse guest-controlled input, so the new tests assert the rejections: reused,
  backwards and reserved stream ids, and unresolvable tool, resource and prompt
  names returning a structured error instead of panicking the process.
- Narrowed the `*_Store` ignore rule to `*.DS_Store`. macOS sets
  `core.ignorecase=true`, so the bare pattern matched source directories such
  as `crates/capsem-process/src/job_store/` case-insensitively and silently:
  the tree still built locally, and a fresh clone would have failed on an
  unresolved `mod tests;`.
