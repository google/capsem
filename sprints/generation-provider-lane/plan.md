# Generation Provider Lane

## What

Build a reusable Rust AI/model lane that can serve UI artifact generation,
future plugin APIs, and security-engine dynamic review without embedding
provider HTTP code in the UI demo server.

## Decisions

- Capsem owns the typed generation contract.
- Provider HTTP belongs to an external Rust model crate; start with `siumai`
  because it covers text, image, speech, transcription, and video families.
- The prototype reads a minimal Capsem-compatible `service.toml` credential
  shape so the real tree can swap in `capsem-core` settings without changing
  the generation API.
- Provider credentials resolve from Capsem service credentials first, then
  environment variables as a developer fallback.

## Files

- `crates/capsem-ai`: reusable model-use engine, settings resolver,
  fake provider, and Siumai provider.
- `crates/capsem-plugin-server`: server route adaptation only.
- `crates/capsem-ui-catalog`: generated text artifact shape.
- `python/capsem_native_client`: rehearsal client method for text generation.
- `sprints/generation-provider-lane`: plan and tracker.

## Done

- The server has no custom Gemini HTTP implementation.
- Text generation and image generation share one `capsem-ai` engine.
- Gemini key can come from Capsem-style settings credential
  `google-api-key`.
- Tests prove settings credential resolution, fake-provider generation, server
  route behavior, and Python client mapping without live provider calls.

## Proof Matrix

- Unit/contract: settings parser, credential resolution, fake provider.
- Functional: server routes build native artifacts through `capsem-ai`.
- Adversarial: missing credential returns typed `configMissing` artifact.
- E2E/VM: deferred until porting into main Capsem service.
- Telemetry: existing native telemetry captures operation/status.
- Performance: no benchmark in this slice; provider call latency is external.
