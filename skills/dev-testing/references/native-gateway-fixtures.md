# Native gateway fixtures

Gateway marker files are prepared empty before sandbox confinement. Their
existence is not readiness: the gateway writes ports first and the complete
64-character ASCII alphanumeric token last. Startup fixtures must check that
published state before parsing ports or probing HTTP. Cover empty and partial
real-file states directly; keep the existing bounded readiness poll and sandbox
authority intact.

Bind native test-binary authorization to the immutable startup environment
that selected its path (`host_binary_environment()`), rather than re-reading
per-test environment state. Synthetic gate-plan tests deliberately clear
release markers; that isolation must not strip a runtime fixture's selected
package provenance or trigger a source rebuild inside release qualification.
