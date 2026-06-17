"""Model IDs used by Ironbank model-client fixtures and canaries."""

from __future__ import annotations

HERMETIC_LOCAL_OLLAMA_MODEL = "gemma4:latest"
HERMETIC_OPENAI_COMPAT_MODEL = HERMETIC_LOCAL_OLLAMA_MODEL
HERMETIC_OPENAI_PRICED_MODEL = "gpt-5-nano"
HERMETIC_ANTHROPIC_MODEL = "claude-sonnet-4-6"
HERMETIC_GEMINI_MODEL = "gemini-2.5-flash"
HERMETIC_AGY_MODEL = "gemini-3.5-flash-low"

LIVE_OPENAI_RESPONSES_MODEL = "gpt-5-nano"
LIVE_OPENAI_IMAGE_MODEL = "gpt-5.5"
LIVE_OPENAI_EMBEDDING_MODEL = "text-embedding-3-small"
LIVE_GEMINI_TEXT_MODEL = "gemini-3.5-flash"
LIVE_GEMINI_IMAGE_MODEL = "gemini-3.1-flash-image-preview"
LIVE_CLAUDE_MODEL = "claude-sonnet-4-6"
