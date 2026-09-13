# Manual end-to-end scenarios

These are hands-on demonstrations, not part of any gate. They boot real VMs
against the locally built binaries and print a human-readable PASS/FAIL
report. Nothing here is collected by pytest (the filenames are not `test_*`)
and no gate suite runs this directory, so they never touch CI timing.

They exist because some behaviour is best shown as a live scenario a person
can watch and re-run: a sandboxed agent going at a service over a private
network, and a sandboxed VM reaching a local model through capsem's egress.

## Prerequisites

- Build and codesign the host binaries once (they land in
  `cache/target/cargo/debug`, which the scenarios read):

  ```bash
  just _sign
  ```

- The Redis image fixture must be materialized (any kingslanding run does
  this; otherwise `just focus-test kingslanding slow` once).
- Each scenario stands up its own throwaway `capsem-service` on a private
  socket under a temp home. It never registers or mutates an installed
  service. Run it under the build-system interpreter, and wrap it in the
  bounded-command helper so a hang cannot leak a VM:

  ```bash
  python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 1200 \
      -- uv run --project build_system --frozen python tests/manual/<script>.py
  ```

## Scenarios

### `cyber_gym.py` — agent VM attacks a target over a private network

Boots two container VMs on one named network. The "target" runs a TCP
listener (the guest's own `capsem-bench-rs`); the "agent" attacks it and the
script asserts five properties:

1. **Reachability** — the agent reaches the target on its private address.
2. **Connection churn** — 200 short-lived private connections in a row, every
   one answered. This is the exact pattern that used to lose ~1 reply in 2000
   before the IPC channel released its socket descriptor in the wrong order
   (fixed in `capsem_foundation::ipc_channel`); a CTF agent hammering a
   service reproduces that churn, so this is the live regression check.
3. **Private DNS** — `target.<network>.capsem.internal` resolves to the
   member address, looked up through the container's own resolver.
4. **Isolation** — a third VM that never joined the network reaches nothing.
5. **Audit** — the network ledger admits and records the agent→target flow
   from both ends.

For a real capture-the-flag, swap the target for a vulnerable image and give
the agent real tooling. TCP connect-style attacks work; the switch drops raw
packets (it refuses IP protocol 6 on the link), so SYN scans and packet
crafting will not reach the target — connect scans will.

### `vm_ollama.py` — a sandboxed VM prompts a local model through capsem

Boots one sandboxed VM that holds no model weights and has no open internet.
Its only route to a completion is capsem's egress: the guest redirects any
connection to `:11434` into its net-proxy, the host proxy forwards it to the
Ollama daemon on the host, and the `ai_ollama_*` profile rules admit and
record the call. The script then:

1. Prompts the model from inside the VM and prints the answer.
2. Reads the VM's own `session.db` and asserts capsem recorded the call — a
   `model_calls` row with `protocol = 'ollama'`, or the raw `net_events` row
   on port 11434.

Requires a host Ollama serving the model. Point it at whatever `ollama list`
shows:

```bash
CAPSEM_GYM_MODEL=gemma4 \
CAPSEM_GYM_PROMPT="In one sentence, say hello and name the capital of France." \
    uv run --project build_system --frozen python tests/manual/vm_ollama.py
```

Defaults: `CAPSEM_GYM_MODEL=gemma4`, a short built-in prompt.

### `ctf_gemma.py` — a local model drives an attack the sandbox observes

Closes the loop between the two scenarios above: a local model is the brain, a
sandboxed VM is the hands. It boots an agent VM and a target VM on one private
network; the target serves a random secret flag over HTTP on its private
address (hidden from the model). Each turn the model (reached through the
`:11434` egress) is shown the target and its last command's output and must
reply with one shell command or `SOLVED: <flag>`. The command runs inside the
agent's container namespace, over the private network, and the output feeds
back.

The run passes only if **both** hold, because either alone proves nothing:

1. the model-driven agent captured the flag, and
2. capsem's network ledger recorded the agent→target TCP flow — a
   `network.connect` event, `context: private`, `protocol: tcp`, admitted,
   naming both VMs and the target port.

Every model turn (prompt, command, output) and the full raw ledger events are
written to a temp-dir file (`<tmp>/capsem-ctf-evidence/ctf-<model>-<stamp>.json`; override with `CAPSEM_GYM_EVIDENCE_DIR`), so
the run is inspectable afterwards. A run you cannot audit proves nothing.

The challenge is deliberately trivial (flag on `GET /`) to demonstrate the
loop, not the model's strength. Point `CAPSEM_GYM_MODEL` at a stronger local
model (e.g. a large local Qwen) and raise `CAPSEM_GYM_TURNS` for a real
capability signal against a harder target.

```bash
CAPSEM_GYM_MODEL=gemma4 CAPSEM_GYM_TURNS=6 \
    uv run --project build_system --frozen python tests/manual/ctf_gemma.py
```
