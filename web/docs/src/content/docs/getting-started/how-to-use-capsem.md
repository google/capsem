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

To get a workspace with Claude Code available, launch this VM:

```sh
capsem create -n my-work --image claude-code
```

- `-n my-work` names the VM and makes it **persistent**: you can stop and
  resume it without losing your files or setup. You can also
  [suspend and resume](#pause-your-work-and-return-later) to continue running
  programs where you left off, or [duplicate the setup with `fork`](#duplicate-a-setup)
  to try changes in a separate VM. Use `my-work` to select this VM in commands.
- `--image claude-code` chooses the container to run inside the VM. You can
  use one of Capsem's prebuilt containers, such as this one with Claude Code,
  or your own container if your administrator authorizes it. See
  [Choose a container](#choose-a-container) for the available images and how
  to use your own.

Capsem downloads the image if needed and starts the VM in the background.
The next step is to open it.

## Access your VM

You can work in the VM from your terminal or from the graphical Capsem app.
Both access the same VM; you can switch between them.

### From your terminal

Open the terminal interface (TUI), focused on `my-work`:

```sh
capsem shell my-work
```

Type in its terminal to run commands inside the VM. With several VMs open,
use **Alt + Left/Right** to move between them. **Alt + Q** closes the TUI;
your VM keeps running, and you can reopen it with the same command.

### From the graphical app

Open **Capsem** (in Applications on macOS), or choose **Dashboard** from
the Capsem system tray menu. Click `my-work` in the session list to open it,
then use its **Terminal** view to work inside the VM. The same VM tab also
lets you inspect files, logs, and activity.

## Pause your work and return later

Suspend the VM when you want to free up its CPU and memory without losing
where you are in your work. Run this in another terminal on your computer:

```sh
capsem suspend my-work
```

Your programs are paused so you can continue where you left off. When you
are ready to work again, resume the VM and reopen its terminal:

```sh
capsem resume my-work
capsem shell my-work
```

Naming the VM with `-n` is what lets you keep and suspend it.

## Duplicate a setup

Once you have prepared a project and set up your tools in `my-work`, you
can reuse that setup instead of doing it again. **Fork** makes a new VM with
a copy of your files and setup. Use it to try a change while keeping the
original ready to return to.

For example, make a copy named `experiment`, start it, and open its terminal:

```sh
capsem fork my-work experiment
capsem resume experiment
capsem shell experiment
```

`my-work` is the VM to copy; `experiment` is the name of the new persistent
VM. You can change files and try different tools in `experiment` without
changing `my-work`. Programs start afresh in the copy; use suspend and resume
when you want to continue programs already running in the original.

## Choose a container

Choose a different container when you want another agent or a workspace
without an agent. See which images are available to you with:

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

For example, to work with Codex instead of Claude Code, create a new VM and
open it:

```sh
capsem create -n coding --image codex-cli
capsem shell coding
```

If your team already has a container with the tools you need, you can use
it instead of one from the list. Your administrator must authorize that image
and the registry it comes from:

```sh
capsem create -n custom --image ghcr.io/your-org/your-image:tag
```

Replace `ghcr.io/your-org/your-image:tag` with your team's authorized image.
For an image on Docker Hub, use `docker://`, for example `docker://alpine:3`.
Custom images are blocked by default, and your administrator may also limit
the images available in the catalog.

## Advanced usage

### Temporary VMs

For a quick experiment you do not need to keep, omit `-n`:

```sh
capsem create --image claude-code
capsem list
```

`create` prints a VM ID, and `list` shows your VMs and their IDs. Replace
`<VM-ID>` below with it to open the VM and, when finished, delete it:

```sh
capsem shell <VM-ID>
capsem delete <VM-ID>
```

An unnamed VM cannot be suspended. Closing its terminal leaves it running;
deleting it removes its files, so copy out anything you want to keep first.

For a quick check that should clean up after itself, `run` creates a VM,
executes a command, and deletes the VM when that command finishes. This
example prints the Linux system information using the `dev` container:

```sh
capsem run --image dev sh -c 'uname -a'
```

### CPU and memory

Use fewer CPU cores and less memory to leave more of your computer available
for other work, or allocate more for demanding tasks. Choose these resources
when creating the VM:

```sh
capsem create -n my-project --cpu 2 --ram 8 --image claude-code
```

`--cpu 2` gives the VM two CPU cores; `--ram 8` gives it 8 GB of memory.
Choose values your computer can spare and that your administrator permits.
Put these options before `--image`; anything after the image is a command
to run in the container.

## Next steps

- [Guides](/guides/) for task-based how-tos.
- [Reference](/reference/) for the complete CLI and MCP contracts.
- [SDK](/sdk/) to manage VMs from your application.
