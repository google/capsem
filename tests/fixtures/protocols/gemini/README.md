# Gemini Protocol Fixtures

Gemini API Ironbank tests use deterministic responses from
`scripts/mock_server_impl.py` for:

- `:streamGenerateContent` with function-call and function-response turns.
- `:generateContent` non-streaming text generation.

Keep recorded or replay-only Gemini API payloads in this directory when a test
needs fixed fixture data instead of generated mock-server responses.
