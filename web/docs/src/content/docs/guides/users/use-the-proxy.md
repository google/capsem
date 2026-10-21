---
title: Block a model response with proxy mode
description: How do I use an OpenAI client with Ollama through Capsem and block a response containing London?
---

Send an OpenAI client's requests through `capsem proxy` to check model
responses before they reach your application. This guide uses Ollama on your
computer and a rule that blocks responses containing `London` while allowing
`Paris`. No VM is created. You need Capsem 0.7.1 running, Ollama, and a local
model; the example was tested with `gemma4:latest`.

## Start Ollama

If Ollama is not already running, start it in a terminal:

```sh
ollama serve
```

In another terminal, run `ollama list` to find an installed model. The steps
below use `gemma4:latest`; replace that name with your model. The built-in
Capsem provider `ollama` points to `http://127.0.0.1:11434`.

## Add the response rule

Add these rules to `~/.capsem/access-rules.toml`:

```toml
[profiles.rules.block_london]
name = "block_london"
action = "block"
priority = 10
match = 'model.response.body.contains("London")'
reason = "Responses mentioning London are blocked."

[profiles.rules.allow_local_ollama]
name = "allow_local_ollama"
action = "allow"
priority = 20
match = '(http.host == "127.0.0.1" && tcp.port == "11434") || model.provider == "ollama"'
```

The first rule checks the model's response body, not the prompt. Its match is
case-sensitive. The second admits this local Ollama endpoint and its model
events; it does not grant arbitrary local-network access. The lower priority
number puts the London block first. Corp policy can still refuse the request.

In `~/.capsem/settings.toml`, select that file:

```toml
[rule_files]
enforcement = "access-rules.toml"
```

If you already have an enforcement file, append the rules there instead and
keep its existing selection. Merge into an existing `[rule_files]` section;
do not duplicate that section. Start a new proxy after saving the rules.

## Start the proxy

Run this in a separate terminal and leave it running:

```sh
capsem proxy --provider ollama --port 18080
```

`--provider ollama` selects the local upstream; `--port 18080` selects your
proxy's port. The command prints its base URL. For Ollama it has no path suffix:
the OpenAI client below adds `/v1` to use Ollama's OpenAI-compatible API.
Keep the default loopback bind; this listener is plain HTTP without authentication.

## Compare Paris and London

Install the OpenAI Python client:

```sh
python -m pip install openai
```

Run this in a notebook or IPython. `api_key="ollama"` is a non-secret placeholder
required by the OpenAI client; local Ollama does not require a provider key.
If you changed the proxy port, change the base URL too.

```python
from openai import AsyncOpenAI

client = AsyncOpenAI(
    base_url="http://127.0.0.1:18080/v1",
    api_key="ollama",
    max_retries=0,
)

response = await client.chat.completions.create(
    model="gemma4:latest",
    messages=[{"role": "user", "content": "Reply with exactly one word: Paris. No explanation."}],
    temperature=0,
    max_tokens=256,
    stream=False,
)
print(response.choices[0].message.content)
```

This prints `Paris` when the model follows the prompt. Now repeat the call with
`London`:

```python
response = await client.chat.completions.create(
    model="gemma4:latest",
    messages=[{"role": "user", "content": "Reply with exactly one word: London. No explanation."}],
    temperature=0,
    max_tokens=256,
    stream=False,
)
```

The OpenAI client raises an `APIStatusError` with HTTP 403. Its response body
identifies the rule:

```text
capsem: model response blocked by security rule: profiles.rules.block_london
```

Keep `stream=False` for this example so the engine evaluates the collected
response before returning it. The rule inspects the response JSON, including
metadata; it is not a semantic check of the visible answer alone. If your model
does not produce `London`, the rule has nothing to block.

## Recorded result

The test ledger recorded these `net_events` rows; `model_calls` also recorded
both statuses for provider `ollama`, model `gemma4:latest`:

| Model reply | Recorded decision | HTTP status | Matched rule |
| --- | --- | --- | --- |
| `Paris` | `allowed` | 200 | `profiles.rules.allow_local_ollama` |
| `London` | `denied` | 403 | `profiles.rules.block_london` |

The standalone command does not expose a history query or record-export
option. These rows were checked in preserved test evidence after worker
shutdown; they are not output from a proxy history command.

## Finish

Close the model client with `await client.close()`, then press Ctrl-C in the
proxy terminal. Capsem closes the listener and removes the proxy's temporary
records. Proxy mode has no retained-history option and sees only requests sent
to this endpoint; use a [VM](/getting-started/how-to-use-capsem/) to isolate the
agent's files, commands, and other traffic.

Use the [Proxy reference](/reference/proxy/) for other providers, flags, and
lease control. To write another response condition, use the
[CEL field and syntax reference](/reference/cel/).
