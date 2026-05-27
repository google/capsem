# Sprint: Profile V2 HTTP/CEL/Builder Hardening

## Tasks

- [x] Create sprint plan and tracker.
- [x] Audit HTTP decompression code paths.
- [x] Add focused CEL edge tests.
- [x] Inspect builder config/defaults/image generation path.
- [x] Add builder guard tests if a gap is found.
- [x] Run focused verification.
- [x] Update changelog/tracker.
- [x] Commit functional milestone.

## Notes

- Starting after legacy policy config removal commit `1278024f`.
- HTTP response decompression has two paths:
  - production streaming path: `handle_request` strips gzip response headers
    and seeds `DecompressionHook` only when `Content-Encoding` contains gzip;
  - direct model-policy helper path: model response tests may pass raw gzip
    bytes, so `policy_v2_model` decodes by gzip magic before parsing.
- Hardened the production path to detect gzip in comma-separated
  `Content-Encoding` token lists, case-insensitively.
- Hardened gzip header parsing to treat RFC 1952 reserved FLG bits as
  malformed and pass bytes through instead of classifying and dropping them.
- CEL parser bug found and fixed: method-like text such as `.contains(` inside
  quoted string literals was being parsed as a method call before comparison
  parsing got a chance.
- Builder audit: `load_guest_config` feeds both Dockerfile context and
  `generate_defaults_json`; existing conformance already checks on-disk
  defaults/mock data. Added a guard that disabled AI providers are excluded
  from rootfs install packages while still described in generated settings.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-core
  net::mitm_proxy::decompression_hook::tests --lib -- --nocapture`; `cargo
  test -p capsem-core policy_v2_cel --lib -- --nocapture`; `uv run python -m
  pytest tests/test_config.py::TestGenerateDefaultsJsonConformance
  tests/test_docker.py::TestGuestConfigBuildAndDefaultsAlignment -q`.
- Functional: `cargo test -p capsem-core policy_v2_ --lib -- --nocapture`;
  `cargo check -p capsem-core`.
- Adversarial: malformed gzip reserved flags pass through; method-looking CEL
  text inside string literals no longer escapes validation; missing fields do
  not satisfy `!=` rules.
- E2E/VM: deferred; no VM boot required for this helper-level hardening slice.
- Telemetry: existing Policy V2 MITM telemetry assertions stayed green in
  `policy_v2_`.
- Performance: not in scope unless code changes require it.
- Missing/deferred: no full `just smoke` / `just test` in this turn.
