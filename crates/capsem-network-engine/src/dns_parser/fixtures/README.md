# dns_parser fixtures

Raw DNS wire-format byte fixtures used as deterministic parse-test
inputs and as seeds for the cargo-fuzz target (`fuzz/fuzz_targets/`).

Each `.bin` file is the on-the-wire encoding of one DNS message, in
network byte order, exactly as a recursive resolver or proxy would
see it. No length prefix, no envelope -- bytes only.

| File | What |
|------|------|
| `simple_a_query.bin` | Standard `A anthropic.com.` query, id=0x1234, RD=1 |
| `aaaa_query.bin` | `AAAA anthropic.com.` query, id=0x4242 |
| `txt_query.bin` | `TXT example.com.` query |
| `mx_query.bin` | `MX example.com.` query |
| `caa_query.bin` | `CAA example.com.` query (qtype 257, rare in the wild) |
| `https_query.bin` | `HTTPS example.com.` query (RFC 9460 SVCB; ECH-relevant) |
| `multi_question_query.bin` | Two-question query (`first.com.` + `second.com.`); RFC-legal but resolver-rare |
| `nxdomain_response.bin` | NXDomain response synthesized by `build_nxdomain` for `blocked.example.com.` |
| `servfail_response.bin` | ServFail response synthesized by `build_servfail` |
| `truncated_query.bin` | Query truncated mid-label -- parse must error, not panic |
| `compression_self_loop.bin` | Hand-crafted message whose name label is a 2-byte pointer to its own offset (RFC 1035 sec 4.1.4 pointer); parser must terminate without infinite loop |
| `header_only.bin` | 12-byte header with all-zero counts; parse returns "no questions" |
| `lying_qdcount.bin` | Header claims qdcount=5 with no question section following |

## Regenerating

The fixtures are checked in and committed. To regenerate after a
hickory-proto upgrade or test data change:

```sh
cargo test -p capsem-network-engine dns_parser::tests::regenerate_fixtures -- --ignored
```

The regen test rebuilds each fixture from a deterministic seed
(fixed transaction ids, fixed names) and writes them back to this
directory. Hand-crafted adversarial fixtures (`compression_self_loop.bin`,
`lying_qdcount.bin`) live as raw byte literals in the regen
function.

The deterministic round-trip test
(`tests::fixtures_roundtrip_through_parse_query`) loads each
fixture via `include_bytes!()` at compile time so test runs don't
hit the filesystem.
