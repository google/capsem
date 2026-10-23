# Native gateway HTTP telemetry acceptance

The canonical selector is:

```text
tests/ironbank/kingslanding/test_gateway_telemetry.py
```

Run it through the normal Ironbank owner against one verified host cohort,
including its rebuilt `capsem-mock-server`. No VM or external collector is
required. The native service and gateway still install their actual platform
confinement; this is not an in-process router fixture.

The existing mock launcher passes `--otlp-http-fixture` explicitly. Without it,
`/otlp/v1/metrics` and `/otlp/summary` return 404. The POST collector requires
`Content-Type: application/x-protobuf`, no compression or query, and the public
synthetic header `Authorization: Bearer capsem-test-collector`. These values are
fixture data, never real credentials. The native recipe sets its isolated
service's OTLP base endpoint to `<mock-base>/otlp` and the encoded header setting
`Authorization=Bearer%20capsem-test-collector`.

The fixture checks the production Rust gateway admission schema and decodes
with the existing generated Rust OTLP types. It retains only the latest bounded
normalized metric points, batch/byte counters and an authorization-verified
boolean. It does not expose or log raw protobuf, headers, request paths, or
payload-derived rejection errors. The service's independent non-gateway metrics
are rejected, not relabeled as gateway evidence. Summary histogram values are
sample counts, not invented duration or CPU measurements.

The enabled test sends genuine gateway requests: health/status, unauthorized and
foreign-host rejections, CORS, and 100 distinct private paths/custom methods.
It verifies exact normalized request/completion counts, readiness/body sample
presence, bounded series, and actual service/gateway log canaries. A graceful
service stop and restart proves a new gateway exporter starts with new series;
old unknown-route samples do not reappear. The disabled test proves normal
service grants keep working with no gateway collector batches. Malicious peer,
sequence, partial-exchange cancellation, wire bounds and stale epoch cases also
have focused Rust coverage; these are not inferred from this native selector.

The confined child intentionally inherits no OTEL configuration. Its ordinary
SDK reader retains the standard 60-second cadence, so each enabled phase has a
90-second admission deadline. No test-only production timer or flush API exists.
Native execution is separate from source-only collection/tests. These assertions
are functional evidence, not an endpoint benchmark, native visual latency,
per-request CPU result, or the two-baseline/two-candidate overhead proof.
