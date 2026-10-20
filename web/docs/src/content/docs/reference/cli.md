---
title: CLI reference
description: What commands, arguments, and options does the Capsem CLI support?
---

This reference describes Capsem's command-line operations for creating,
accessing, managing, and inspecting VMs, plus service administration and
diagnostics. Run these commands on your computer. They accept a VM name or immutable
ID where `VM` appears below; commands explicitly taking `NAME` require a
persistent VM name. Use `capsem COMMAND --help` for that installed version's
complete option help.

## Create and access

| Command | Contract |
| --- | --- |
| `capsem create [OPTIONS] --image IMAGE [CMD…]` | Create a detached VM. Naming it makes it persistent. |
| `capsem shell [VM]` | Open the TUI; optionally focus one VM. Creates and deletes nothing. `-n NAME` also selects a name. |
| `capsem exec VM COMMAND [--timeout SECONDS] [--target workload\|vm]` | Execute a shell command. Image VMs default to their workload; otherwise the VM. Returns the command's exit code. |
| `capsem run [OPTIONS] --image IMAGE [CMD…]` | Run the image's workload in a new temporary VM; remove the VM afterward. |
| `capsem run 'COMMAND'` | Run a shell command in a new temporary VM; remove the VM afterward. |

Put options before `--image`: everything after its image reference is the
workload command, including values beginning with `--`.

| Create/run option | Meaning / default |
| --- | --- |
| `-n, --name NAME` | Create only: persistent VM name. Omitted: temporary VM. |
| `--cpu N` | CPU cores; default 4. |
| `--ram N` | Memory in GiB; default 12. |
| `-e, --env KEY=VALUE` | Repeatable. Container environment with an image; VM environment without one. |
| `--network NAME` | Repeatable named private network. Shell-only `capsem run` does not support it. |
| `--from NAME` | Create only: clone a persistent VM; optionally select a new image. |
| `--timeout SECONDS` | Run only: maximum workload duration. |
| `-p, --publish HOST:GUEST` | Publish a loopback port over vsock; host port 0 selects a free port. Requires an image. |
| `--registry-user USER` | Registry username; password/token comes from `CAPSEM_REGISTRY_PASSWORD`. |
| `--registry-ca PATH` | Additional PEM CA trusted for that registry pull only. |

Image references are catalog names from `capsem images`, `docker://IMAGE`
for Docker Hub, or registry-qualified references. Admission policy still
applies. See [Build your own image](/guides/enterprise/build-an-image/).

## Retain and remove

| Command | Effect |
| --- | --- |
| `capsem suspend VM` | Save running state and release CPU/RAM; persistent VMs only. |
| `capsem resume NAME` | Resume saved state, start a retained VM, or attach to one already running. Alias: `capsem attach`. |
| `capsem restart NAME` | Reboot a persistent VM. |
| `capsem fork VM NAME [-d DESCRIPTION]` | Copy files/setup into another persistent VM; does not copy running programs. |
| `capsem persist VM NAME` | Give a running temporary VM a persistent name. |
| `capsem delete VM` | Delete the VM and its retained state. Alias: `capsem rm`. |
| `capsem purge [--all]` | Remove temporary VMs; `--all` includes persistent VMs and requires confirmation. |

`capsem stop` stops the background service; it is not a per-VM stop command.
Use the UI or SDK's VM stop operation when you want a retained VM shut down.

## Inspect and transfer

| Command | Options / behavior |
| --- | --- |
| `capsem list` | `-q, --quiet`: IDs only. Alias: `capsem ls`. |
| `capsem info VM` | `--json`: typed VM information. |
| `capsem logs VM` | Serial and process logs; `--tail N`. |
| `capsem history VM` | `--tail N` defaults to 500; `--all`, `--search TEXT`, `--layer all\|exec\|audit`, `--json`. |
| `capsem cp SRC DST` | Exactly one side is `VM:PATH`; the other is a local path. Download to `-` for stdout. |
| `capsem images` | Image catalog. `capsem images --help` lists prefetch options. |
| `capsem assets status [--json]` | Runtime asset readiness. |
| `capsem assets ensure [--json]` | Repair missing/corrupt runtime assets. |

File transfer requires a running VM's security ledger. Relative paths select
its workspace; exact guest path handling is specified by the file API.

## Private networks

| Command | Effect |
| --- | --- |
| `capsem network create NAME` | Create a named network. |
| `capsem network list` | List networks. |
| `capsem network inspect NETWORK` | Identity and members. |
| `capsem network connect VM NETWORK` | Attach a VM. |
| `capsem network disconnect VM NETWORK` | Detach a VM. |
| `capsem network delete NETWORK` | Retire an empty network. |
| `capsem network logs NETWORK` | Audit records; `-f`, `--limit`, `--vm`, `--type`, `--decision`. |

Member traffic uses any protocol and bypasses HTTP inspection.
See [network connections](/architecture/security/#network-connections).

## MCP and standalone proxy

`capsem mcp servers` lists configured servers; `capsem mcp tools [--server NAME]`
lists tools; `capsem mcp refresh` rediscovers tools.
`capsem mcp call SERVER__TOOL --args '{}'` sends JSON arguments through policy.
See the [MCP reference](/reference/mcp/).

`capsem proxy [--provider PROVIDER] [--bind IP] [--port PORT]` starts a VM-free
model endpoint. Defaults: provider `openai`, bind `127.0.0.1`, and an available
port. Keep the command running; Ctrl-C stops it and removes its temporary
records. The separate model listener is unauthenticated HTTP.
Use the [Proxy reference](/reference/proxy/) for provider routing and lease
control, or [block a model response](/guides/users/use-the-proxy/) with the
OpenAI/Ollama example.

## Service and updates

| Command | Effect |
| --- | --- |
| `capsem status` | Service and asset readiness. |
| `capsem start` / `capsem stop` | Start/stop the background service. |
| `capsem install` | Register the service. The native package installer normally does this. |
| `capsem version` | Installed version and build information. |
| `capsem update --check` | Refresh update status without applying changes. |
| `capsem update --yes [--channel stable\|nightly]` | Apply the complete installed package/runtime update. |
| `capsem update --corp URL` | Provision corporate configuration; cannot combine with `--assets`. |
| `capsem update --assets [--manifest URL]` | Diagnostic runtime-asset repair or corporate asset selection. |
| `capsem uninstall` | Remove service, binaries, and data; requires confirmation unless `--yes`. |

Manifest/corp inputs are URLs. Local files require `file:///absolute/path`.
Corporate channel selection is locked; see [Manifest reference](/reference/manifests/).

## Diagnostics

`capsem doctor [--bundle]` runs diagnostics in a fresh VM.
`capsem debug` (alias of `capsem support-bundle`) writes a redacted support
archive. Options include `--output`, `--sessions` (default 3, maximum 10),
`--max-session-bytes` (default 50 MiB), and `--include-rootfs`.
`--no-redact` disables redaction.

`capsem completions bash|zsh|fish|powershell` prints shell completions.
