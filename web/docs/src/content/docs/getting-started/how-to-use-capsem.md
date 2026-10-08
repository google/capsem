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

Start a VM to work with an AI agent, pause it when you are done for the day,
or duplicate it to try something new. Run the Capsem commands below in a
terminal on your own computer, after [installing Capsem](/getting-started/installation/).

## Launch your first VM

After installation, create a VM with Claude Code available:

```sh
capsem create -n my-work --image claude-code
```

- `-n my-work` names the VM and makes it **persistent**: you can stop and
  resume it without losing your files or setup. You can also
  [pause and resume running work](#pause-your-work-and-return-later) or
  [fork your VM](#fork-your-vm). Use `my-work` to
  select this VM in commands.
- `--image claude-code` chooses the image that provides the VM's software.
  You can use one of Capsem's prebuilt images, such as this one with Claude Code,
  or your own container image if your administrator authorizes it. See
  [Choose an image for your VM](#choose-an-image-for-your-vm) for the available images and how
  to use your own.

Capsem downloads the image if needed and starts the VM in the background.
Once creation finishes, [open the VM](#access-your-vm) to begin working.

## Access your VM

Once your VM is created, you can access it through the terminal interface
(TUI) or the graphical interface (Capsem UI). Choose either option below;
both open the same VM, so you can switch between them later.

### From your terminal

Open the terminal interface (TUI), focused on `my-work`:

```sh
capsem shell my-work
```

Once the TUI opens, type in its terminal to run commands inside the VM.
With several VMs open, use **Alt + Left/Right** to move between them.
When you want to close the TUI, press **Alt + Q**; your VM keeps running,
and you can reopen it with the same command.

### From the Capsem UI

Open **Capsem** (in Applications on macOS), or choose **Dashboard** from
the Capsem system tray menu. Click `my-work` in the list of VMs to open it,
then use its **Terminal** view to work inside the VM. The same VM tab also
lets you inspect files, logs, and activity.

## Pause your work and return later

When you are ready to pause your work, suspend the VM to free up its CPU
and memory while keeping your running programs ready to continue. Run this
in another terminal on your computer:

```sh
capsem suspend my-work
```

Once suspension finishes, your programs are paused. When you are ready to
work again, resume the VM, then reopen its terminal:

```sh
capsem resume my-work
capsem shell my-work
```

After resuming, you can also return through the [Capsem UI](#from-the-capsem-ui).

## Fork your VM

After setting up your tools and files in `my-work`, you can reuse that VM's
setup instead of doing it again. **Fork** duplicates your VM's files and setup
in a new VM so you can try a change while keeping the original VM.

For example, duplicate `my-work` into a VM named `experiment`, start the
new VM, and open its terminal:

```sh
capsem fork my-work experiment
capsem resume experiment
capsem shell experiment
```

`my-work` is the original VM; `experiment` names the new persistent VM.
Once the new VM is running, you can change files and try tools there without
changing `my-work`. Programs start afresh in the new VM; to continue programs
already running in the original, use [suspend and resume](#pause-your-work-and-return-later).

## Choose an image for your VM

When creating your next VM, choose a different image if you want another
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

## Advanced usage

The following options let you tailor new VMs to other tasks. Choose them
before creating each VM.

### Temporary VMs

For a quick experiment you do not need to keep, omit `-n`:

```sh
capsem create --image claude-code
capsem list
```

After creation, `list` shows your VMs and their IDs. Replace `<VM-ID>` with
the ID printed by `create`, then open the VM:

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

For a quick check that should clean up after itself, `run` creates a VM,
executes a command, and deletes the VM when that command finishes. This
example prints the Linux system information in a VM using the `dev` image:

```sh
capsem run --image dev sh -c 'uname -a'
```

### CPU and memory

Use fewer CPU cores and less memory to leave more of your computer available
for other work, or allocate more for demanding tasks. Choose these resources
when creating the VM:

```sh
capsem create -n custom-vm --cpu 2 --ram 8 --image claude-code
```

`--cpu 2` gives the VM two CPU cores; `--ram 8` gives it 8 GB of memory.
Choose values your computer can spare and that your administrator permits.
Put these options before `--image`; a command after the image replaces that
image's startup command.

## Next steps

Once you are comfortable creating and using VMs, choose the next path for
your task:

- [Guides](/guides/) for task-based how-tos.
- [Reference](/reference/) for the complete CLI and MCP contracts.
- [SDK](/sdk/) to manage VMs from your application.
