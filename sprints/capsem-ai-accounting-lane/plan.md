# Capsem AI Accounting Lane

## Goal

Move the isolated prototype's `capsem-ai` crate to a Capsem-owned provider runtime while reusing the clean surface shape we liked from model adapter crates: text, image, and embedding calls behind one Rust trait. Every generated artifact must expose provider, model, token usage, and estimated cost when the provider returns enough data for accounting.

## Decisions

- Discard third-party provider runtime code after the live smoke showed Gemini image response drift.
- Keep the useful API surface: `generate_text`, `generate_image`, `generate_embedding`, `ModelProvider`, and owned request/result structs.
- Use provider-native HTTP endpoints for OpenAI, Gemini, and Anthropic where needed; do not route through an adapter crate.
- Keep image accounting nullable where providers do not return a reliable usage payload.
- Add embeddings now as a first-class API surface because dashboard/search/retrieval work will need the same credentials, provider routing, usage, and cost ledger.
- Add ignored live smoke tests for OpenAI image, Gemini Nano Banana image, and embeddings. They must skip cleanly when credentials are absent.

## Files

- `Cargo.toml`
- `crates/capsem-ai/Cargo.toml`
- `crates/capsem-ai/src/lib.rs`
- `crates/capsem-plugin-server/src/main.rs`
- `crates/capsem-ui-catalog/src/native_deck.rs`
- `python/capsem_native_client/__init__.py`
- `python/tests/test_capsem_native_client.py`
- `CHANGELOG.md`

## Proof Matrix

- Unit/contract: `capsem-ai` fake provider tests for text/image/embedding accounting; native Gemini image parsing tests.
- Functional: plugin server routes return artifacts with usage/cost metadata where available.
- Adversarial: missing credential and empty input remain typed errors.
- E2E/live: ignored smoke tests exercise OpenAI image, Gemini image, and embedding if credentials are configured.
- Telemetry: native telemetry stores usage/cost JSON from generated artifacts for dashboard wiring.
- Performance: no benchmark claim in this sprint; provider overhead is not the point of this lane.

## Done

- `capsem-ai` default provider is `CapsemHttpModelProvider`.
- Text, image, and embedding calls compile through Capsem-owned HTTP code.
- Artifacts and telemetry expose usage/cost fields.
- Tests and changelog are updated.
