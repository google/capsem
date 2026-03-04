---
title: CAPSEM Installation Guide
description: How to install and set up CAPSEM for managing contextual privacy and security policies for AI agents.
---

## Setup a venv (recommended)
While optinal, we recommend using a virtual environment as this will ensure that the dependencies for CAPSEM are isolated from the rest of your system.

```bash
python -m venv .venv
source .venv/bin/activate  # On Windows use `.venv\Scripts\activate`
```

## Install CAPSEM from PyPI

The recommended way to install CAPSEM is to use a python package manager.

```bash
pip install capsem
```

## Configuring environment variables

CAPSEM uses environment variables to configure its behavior. You can set these variables in your shell or in a `.env` file.
You can copy `.env.example` to `.env` and modify it as needed as starting point.

The available environment variables are:
- `GOOGLE_GENAI_USE_VERTEXAI` = "TRUE" use VertexAI for Google GenAI models which is recommended. Set to "FALSE" to use the REST API.
- `GOOGLE_API_KEY` = ""  The Gemini API key or you can also use Application Default Credentials via `gcloud auth application-default login`

- `OPENAI_API_KEY` = "" The OpenAI API key to use OpenAI models

- `ANTHROPIC_API_KEY` = ""  The Anthropic API key to use Claude models

- `VIRUS_TOTAL` = ""  The VirusTotal API key to use VirusTotal for URL and file scanning via the content policy.
