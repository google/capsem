# Policy V2 Microbenchmarks

Scoped Policy V2 closure benchmark for MCP-policy-v2 release prep.

Command:

```bash
cargo bench -p capsem-core --bench policy_v2 -- --sample-size 10 --warm-up-time 0.1 --measurement-time 0.2
```

Sample captured on 2026-05-10:

| Benchmark | Median-ish range |
| --- | ---: |
| `policy_v2_http_request_match` | 1.61-1.76 us |
| `policy_v2_dns_query_match` | 960-967 ns |
| `policy_v2_model_response_match` | 1.32-1.37 us |
| `policy_v2_model_tool_call_match` | 2.11-2.12 us |
| `policy_v2_hook_decision_match` | 1.51-1.52 us |
| `policy_hook_response_decode` | 330-335 ns |
