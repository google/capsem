# Sprint: Capsem AI Accounting Lane

## Tasks

- [x] Plan provider/accounting sprint.
- [x] Replace Siumai/provider-crate runtime with Capsem-owned HTTP provider code.
- [x] Add owned usage and cost structs to the Capsem AI contract.
- [x] Add embedding request/result path.
- [x] Wire server routes, artifact specs, telemetry, and Python client.
- [x] Add ignored live smoke tests for OpenAI image, Gemini image, and embeddings.
- [x] Changelog.
- [x] Testing gate.
- [x] Commit.

## Notes

- Pivot: `liter-llm` had the best surface shape but failed the live Gemini image smoke because its response model did not match the provider output.
- The implementation now keeps the same surface shape but owns the provider HTTP code in `CapsemHttpModelProvider`.
- OpenAI, Gemini, and Anthropic text use native provider request/response structs. Gemini image uses native `generateContent` with image response modalities.
- Image generation stores `usage: null` and `cost: null` when providers do not return a reliable image accounting payload.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-ai` covers fake provider usage/cost, embedding shape, native Gemini image parsing, and missing credential typing.
- Functional: `cargo test -p capsem-ai -p capsem-plugin-server` covers generated text and embedding artifacts through the server provider helpers.
- Adversarial: missing credential and empty embedding input stay typed errors; full malformed route coverage remains deferred.
- E2E/live: ignored live smoke tests exist for OpenAI image, Gemini image, OpenAI embedding, and Gemini embedding; `live_gemini_image_generation_smoke` and `live_gemini_embedding_smoke` passed against local Capsem settings. OpenAI live calls remain blocked by account quota/billing, not by the code path.
- Telemetry: server telemetry copies artifact `usage` and `cost` JSON for generated calls.
- Performance: explicitly deferred; no performance claim
- Missing/deferred: full dashboard visualization of usage/cost remains future Capsem integration work; OpenAI live proof needs a usable OpenAI account/quota.

## Test Log

- `cargo test -p capsem-ai -p capsem-plugin-server`
- `PYTHONPATH=python python3 -m unittest discover -s python/tests`
- `cargo test -p capsem-ai live_gemini_image_generation_smoke -- --ignored --nocapture`
- `cargo test -p capsem-ai live_gemini_embedding_smoke -- --ignored --nocapture`
- `cargo test`
- `git diff --check`
