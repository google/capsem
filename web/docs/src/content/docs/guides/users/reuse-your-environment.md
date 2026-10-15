---
title: Configure and reuse your environment
description: How do I choose an image, retain my tools and files, and duplicate a VM?
---

Choose an image and resources when creating a VM. Name the VM when you want to
keep its files, return to its running programs after suspension, or duplicate
a prepared environment. This guide assumes
[Capsem is installed](/getting-started/installation/).

## Choose an image for your VM

When creating a VM, choose a different image if you want another
agent or a VM without an agent. Before creating it, see which images
are available to you:

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

Once you have chosen an image, pass its name to `--image`. For example,
create a new VM with Codex, then open it:

```sh
capsem create -n coding --image codex-cli
capsem shell coding
```

If your team already has a container image with the tools you need, you can use
it instead. After your administrator authorizes the image and its registry,
create a VM with its full image reference:

```sh
capsem create -n custom --image ghcr.io/your-org/your-image:tag
```

Replace `ghcr.io/your-org/your-image:tag` with your team's authorized image.
For an image on Docker Hub, use `docker://`, for example `docker://alpine:3`.
Custom images are blocked by default, and your administrator may also limit
the images available in the catalog.

## Temporary VMs

For a quick experiment you do not need to keep, omit `-n`:

```sh
capsem create --image claude-code
capsem list
```

After creation, `capsem list` shows your VMs and their IDs. Replace `<VM-ID>` with
the ID printed by `capsem create`, then open the VM:

```sh
capsem shell <VM-ID>
```

When finished, copy out anything you want to keep, close the TUI, and delete
the VM from your computer's terminal:

```sh
capsem delete <VM-ID>
```

An unnamed VM cannot be suspended. Closing its terminal leaves it running;
deleting it removes its files.

For a quick check that should clean up after itself, `capsem run` creates a VM,
executes a command, and deletes the VM when that command finishes. This
example prints the Linux system information in a VM using the `dev` image:

```sh
capsem run --image dev sh -c 'uname -a'
```

## CPU and memory

Use fewer CPU cores and less memory to leave more of your computer available
for other work, or allocate more for demanding tasks. Choose these resources
when creating the VM:

```sh
capsem create -n custom-vm --cpu 2 --ram 8 --image claude-code
```

`--cpu 2` gives the VM two CPU cores; `--ram 8` gives it 8 GiB of memory.
Choose values your computer can spare and that your administrator permits.
Put these options before `--image`; a command after the image replaces that
image's startup command.

## Keep an existing temporary VM

If you started without a name and now want to keep the VM:

```sh
capsem persist <VM-ID> saved-work
```

Replace `<VM-ID>` with its ID from `capsem list`. The new name makes it
persistent; you can now use `saved-work` for suspension and resumption.

## Pause and return

Once a named VM contains your tools and files:

```sh
capsem suspend my-work
capsem resume my-work
capsem shell my-work
```

Suspension preserves running programs while freeing host CPU and memory.
Stopping a named VM through the UI or SDK retains its files but ends its
running programs. `capsem stop` stops the whole service, not one VM.

## Duplicate a prepared VM

After preparing `my-work`, create an independent persistent copy:

```sh
capsem fork my-work experiment
capsem resume experiment
capsem shell experiment
```

Files and setup are copied; programs start afresh. Changes in `experiment`
do not change the original. To create from a saved environment while selecting
another image, use `capsem create --from my-work -n next-work --image codex-cli`.
The clone preserves the workspace and image volumes; the selected image supplies
its software.

## Check what you kept

Use `capsem list` and `capsem info NAME` to confirm identity and state.
Read back a file or run a tool after resuming or starting the clone.
Delete a VM only after copying out work you need.

See the [CLI reference](/reference/cli/) for exact creation/lifecycle options
and [Build your own image](/guides/enterprise/build-an-image/) for a repeatable
team environment.
