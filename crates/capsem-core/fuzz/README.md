# dns_parser fuzz targets

cargo-fuzz / libFuzzer harnesses for the DNS wire-format codec.

## Setup

```sh
cargo install cargo-fuzz       # one-time, requires nightly toolchain
rustup install nightly
```

## Run

From the repo root:

```sh
cd crates/capsem-core/fuzz
cargo +nightly fuzz run parse_query     -- -max_total_time=60
cargo +nightly fuzz run build_nxdomain  -- -max_total_time=60
cargo +nightly fuzz run build_servfail  -- -max_total_time=60
cargo +nightly fuzz run round_trip      -- -max_total_time=60
```

Plan acceptance (T3 mitm-redesign sprint, T3.c slice): each target
must survive 60s without a crash, panic, hang, or out-of-memory.

## Targets

| Target | What it asserts |
|--------|-----------------|
| `parse_query` | `parse_query(&[u8])` returns in bounded time, no panic / OOM, on any input |
| `build_nxdomain` | `build_nxdomain(&[u8])` is safe on any input -- the policy-block path runs this on whatever the guest agent sent |
| `build_servfail` | Same shape as build_nxdomain on a different ResponseCode -- worth a separate target so libFuzzer converges independently |
| `round_trip` | If `parse_query` succeeds, `build_nxdomain` on the same bytes also succeeds AND the response re-parses to a Message whose first question matches the input -- catches divergence between the parse + rebuild paths that would let malformed queries escape NXDOMAIN gating |

## Corpus seeds

Each `corpus/<target>/` directory is pre-seeded with the T3.b
fixtures (`crates/capsem-network-engine/src/dns_parser/
fixtures/*.bin`) so libFuzzer starts with structurally diverse
inputs -- standard A/AAAA/TXT/MX/CAA/HTTPS queries, multi-question,
truncated, header-only, lying-qdcount, compression-self-loop, and
the synthetic NXDomain/ServFail responses. Fast structural coverage
on the first few hundred iterations.

## Triaging a crash

cargo-fuzz writes minimized reproducer files to `artifacts/<target>/`
when a crash trips. Check those in alongside a regression test in
`crates/capsem-network-engine/src/dns_parser/tests.rs` so the bug stays fixed:

```sh
xxd artifacts/parse_query/crash-<sha>          # inspect bytes
cargo +nightly fuzz tmin parse_query artifacts/parse_query/crash-<sha>
# -> writes minimized version to the same dir
```

Then mirror the minimized bytes into a new `fixtures/regression_<id>.bin`
(via the `dns_fixture_gen` example or by hand) and add a
`fixture_regression_<id>_does_not_panic` test. The crash artifact
itself is gitignored; only the minimized fixture + regression test
get committed.
