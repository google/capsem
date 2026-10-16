# Harbor Capsem Sandbox

`harbor-capsem-sandbox` (`harbor_capsem`) provides a [`BaseEnvironment`](https://harborframework.com) backed by [Capsem](https://capsem.org) hardware-isolated micro-VMs for [Harbor](https://github.com/laude-institute/harbor) and [Terminal-Bench](https://www.tbench.ai/).

## Installation

From a `capsem` checkout:

```bash
cd integrations/harbor
uv sync
```

## Usage with Harbor CLI

Run any Harbor / Terminal-Bench task inside a Capsem micro-VM using `--env harbor_capsem:CapsemEnvironment`:

```bash
uv run harbor run \
  --env harbor_capsem:CapsemEnvironment \
  --ek export_trace=true \
  --dataset terminal-bench@2.0 \
  --task-name fix-git \
  --agent terminus-2 \
  --model vertex_ai/gemini-3.5-flash
```

### Environment Kwargs (`--ek`)

- `--ek execution_mode=container` (default): builds `environment/Dockerfile` (or pulls `docker_image` / single-service `docker-compose.yaml`) into a Capsem OCI workload container inside the micro-VM.
- `--ek execution_mode=vm`: boots the default Capsem Linux VM without starting an OCI workload container.
- `--ek broker_credentials=true` (default): registers raw provider API keys (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`, `GOOGLE_API_KEY`) passed to `exec(env=...)` in Capsem's host credential broker (`hypervisor.credentials.inject(provider, value, storage="memory")`) and replaces the guest environment variable with an opaque `credential:blake3:<64-hex>` reference. Capsem's MITM proxy substitutes the real secret on outbound HTTPS requests so raw keys never enter the guest VM or container.
- `--ek export_trace=true` (or `CAPSEM_TRACE_EXPORT=1`): exports `<vm_id>.rows.ndjson.gz` and `<vm_id>.warc.gz` into `trial_dir` at environment teardown (`vm.export.events(...)`), enriches `agent/trajectory.json` with `capsem_events` (`exec_events`, `fs_events`, `net_events`, `dns_events`, `audit_events`, `substitution_events`) and `capsem_model_call` metadata (joining by `turn_id` when `model_calls` are present or by `[step.timestamp, next_step.timestamp)` timestamp windows for host-loop agents), or synthesizes an `ATIF-v1.7` `agent/trajectory.json` via `events_to_atif` (`rows_to_atif`) when the agent did not emit one.
- `--ek include_security_events=false` (default; set `true` or `CAPSEM_TRACE_INCLUDE_SECURITY=1` to retain): omits high-volume `security_rule_events` and `security_decision_events` tables from the exported `<vm_id>.rows.ndjson.gz` unless enabled.
- Managed session lifecycle: `CapsemEnvironment` uses lease-owned managed sessions (`Hypervisor.managed()`), so `stop(delete=False)` raises `NotImplementedError`.

## Model Endpoints & Local Proxy Routing

### 1. Host-Loop Agent (`terminus-2`) with Vertex AI ADC

When running a host-loop agent (`terminus-2`) against Vertex AI via LiteLLM and Google Application Default Credentials (`gcloud auth application-default login`), LLM API calls execute on the host while tool commands execute inside the Capsem micro-VM:

```bash
VERTEXAI_PROJECT=your-gcp-project VERTEXAI_LOCATION=global \
uv run harbor run \
  --env harbor_capsem:CapsemEnvironment \
  --ek export_trace=true \
  --dataset terminal-bench@2.0 \
  --task-name fix-git \
  --agent terminus-2 \
  --model vertex_ai/gemini-3.5-flash
```

### 2. Host-Loop Agent (`terminus-2`) with Local OpenAI-Compatible Proxy (`api_base`)

When routing through a local OpenAI-compatible proxy (`http://127.0.0.1:11434/v1`), pass `--ak api_base=...` and `OPENAI_API_KEY`:

```bash
OPENAI_API_KEY=local-proxy \
uv run harbor run \
  --env harbor_capsem:CapsemEnvironment \
  --ek export_trace=true \
  --dataset terminal-bench@2.0 \
  --task-name fix-git \
  --agent terminus-2 \
  --model openai/google/gemini-3.5-flash \
  --ak api_base=http://127.0.0.1:11434/v1 \
  --agent-env OPENAI_API_KEY=local-proxy
```

### 3. In-VM CLI Agents (`gemini-cli`, `claude-code`, `codex`) & Custom `*_BASE_URL`

When an installed CLI agent executes inside the Capsem container/VM (`CapsemEnvironment.exec`):

- **Default HTTPS Provider Endpoints (`generativelanguage.googleapis.com`, `api.anthropic.com`, `api.openai.com`)**: `broker_credentials=True` rewrites `GEMINI_API_KEY` / `GOOGLE_API_KEY` / `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` into `credential:blake3:<64-hex>`. Capsem's MITM proxy injects the real key upstream and records full `model_calls`, `turn_id`, token usage, and `estimated_cost_usd` in `<vm_id>.rows.ndjson.gz` + `<vm_id>.warc.gz`, enabling exact `turn_id` ATIF trajectory enrichment.
- **Local HTTP Proxy (`OPENAI_BASE_URL=http://127.0.0.1:11434/v1`)**: `capsem-init` redirects plain HTTP on port `11434` (`DEFAULT_HTTP_UPSTREAM_PORTS`) through `:10080` and `vsock:5002` to `capsem-proxy`'s plain-HTTP MITM (`[ai.ollama]`, which maps to `CredentialProvider::OpenAi` for credential substitution). `should_broker_credential` therefore brokers `OPENAI_API_KEY` into `credential:blake3:<64-hex>` on both HTTPS and plain-HTTP port `11434` endpoints so raw API keys never enter the guest environment.
- **In-VM Vertex AI (`*aiplatform.googleapis.com`)**: Capsem's built-in Google provider profile (`default_provider_rules.toml`, `feat/vertex-ai-mitm`) includes `aiplatform.googleapis.com:443` and `*-aiplatform.googleapis.com:443` in `[ai.google].allowed_remote_targets` and `[ai.google.rules.http_gemini_api]`, binding Vertex AI endpoints (`*aiplatform.googleapis.com`) to `ProviderKind::Google` (`ModelProtocol::Google`) by default without manual `settings.toml` overrides. Capsem's credential broker rewrites `Authorization: Bearer credential:blake3:<64-hex>` to the brokered GCP access token upstream and records `model_calls` with `provider = "google"`, `model`, `turn_id`, token usage, and `estimated_cost_usd`.

## Testing

Hermetic unit tests (no live Capsem service required):

```bash
uv run pytest tests/test_environment_unit.py tests/test_credentials_unit.py tests/test_trace_unit.py
```

Live end-to-end acceptance suite across 3 Harbor benchmarks (`terminal-bench@2.0`, `camel-ai/seta-env`, `quixbugs/quixbugs`), in-VM CLI credential brokering, and local proxy/Vertex endpoints (`CAPSEM_LIVE_SELF_CHECK=1`):

```bash
CAPSEM_LIVE_SELF_CHECK=1 uv run pytest -v tests/test_live_acceptance.py
```
