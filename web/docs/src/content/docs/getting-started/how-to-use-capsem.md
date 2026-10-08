---
title: How to use Capsem
description: How do I launch, keep, and customize a Capsem VM?
sidebar:
  order: 2
prev:
  label: Installation
  link: /getting-started/installation/
next: false
---

Run the commands below on your host computer, after installing Capsem.

## Launch your first VM

Create a VM named `my-work` using the Claude Code container:

```sh
capsem create -n my-work --image claude-code
capsem shell my-work
```

Capsem pulls the container image, boots an isolated Linux VM, and starts the
container inside it. `capsem shell my-work` opens the terminal UI for that VM;
it does not create another VM.

The name makes the VM **persistent**: its workspace and disk state are kept
so you can return to your work. Closing the terminal UI does not delete it.

### Suspend and resume

In another host terminal, suspend the VM when you want to pause your work:

```sh
capsem suspend my-work
```

Suspend saves the VM's memory and CPU state to disk. Resume it later with:

```sh
capsem resume my-work
```

Only persistent VMs can be suspended. You can reopen the terminal with
`capsem shell my-work`.

### Fork your work

Create a separate persistent copy of the VM's disk state:

```sh
capsem fork my-work experiment
capsem resume experiment
capsem shell experiment
```

The fork has its own name and disk state, so changes in `experiment` do not
change `my-work`. Fork copies disk state, not the running memory state.
You can also use the fork as a template for another VM:

```sh
capsem create -n another-experiment --from experiment
```

## Choose a container

List the catalog images available under your installation's image policy:

```sh
capsem images
```

The official catalog includes:

| Image | Contains |
| --- | --- |
| `claude-code` | Claude Code |
| `codex-cli` | OpenAI Codex CLI |
| `agy` | Antigravity CLI |
| `dev` | A development toolchain without an AI agent |
| `claude-desktop` | Claude Desktop with a streamed graphical interface (arm64 only) |

Use the catalog name with `--image`:

```sh
capsem create -n coding --image codex-cli
```

Your administrator can restrict or replace the catalog. To use your own OCI
container image, its source and image must be authorized by the image policy:

```sh
capsem create -n custom --image ghcr.io/your-org/your-image:tag
```

Replace the reference with your authorized image. For Docker Hub images, use
`docker://`, for example `docker://alpine:3`. Custom images are refused by the
default policy; using a full image reference does not bypass that policy.

## Advanced usage

### Temporary VMs

Omit `-n` to create a temporary VM:

```sh
capsem create --image claude-code
capsem list
```

Use the returned VM ID instead of a name, for example `capsem shell <VM-ID>`.
An unnamed VM cannot be suspended; do not rely on it to retain your work.
`create` runs detached, so closing its terminal does not automatically delete
it. Delete it when finished with `capsem delete <VM-ID>`.

For a single command, `run` creates a temporary VM and deletes it when the
command exits:

```sh
capsem run --image dev sh -c 'uname -a'
```

### CPU and memory

Set CPU cores and RAM in GB when creating the VM:

```sh
capsem create -n larger --cpu 4 --ram 8 --image claude-code
```

Put Capsem options before `--image`: arguments after the image are treated
as the container command. Choose resources your host can spare and that your
administrator's policy permits.

## Next steps

- [Guides](/guides/) for task-based how-tos.
- [Reference](/reference/) for the complete CLI and MCP contracts.
- [SDK](/sdk/) to manage VMs from your application.
