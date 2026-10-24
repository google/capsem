---
title: How to use Capsem
description: How do I launch, access, stop, and duplicate my first Capsem VM?
sidebar:
  order: 2
prev:
  label: Installation
  link: /getting-started/installation/
next:
  label: Configure and reuse your environment
  link: /guides/users/reuse-your-environment/
---

This guide walks you through creating your first VM, opening it through the
terminal or Capsem UI, pausing your work, and duplicating the VM to try a change.
Run the Capsem commands below in a
terminal on your own computer, after [installing Capsem](/getting-started/installation/).

## Launch your first VM

After installation, create a VM with Claude Code available:

```sh
capsem create -n my-work --image claude-code
```

- `-n my-work` names the VM and makes it **persistent**: you can stop and
  start it again without losing your files or setup, or duplicate your VM with
  [`capsem fork`](#fork-your-vm). Use `my-work` to
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
The TUI manages multiple VMs in one terminal window. Use **Alt + Left/Right**
to switch VM tabs, **Alt + L** to see VMs and their status, and **Alt + ?**
for help. See [all the navigation controls](/guides/users/use-the-terminal/#work-with-multiple-vms).
When you want to close the TUI, press **Alt + Q**; your VM keeps running,
and you can reopen it with the same command.

### From the Capsem UI

Open the [Capsem UI](http://127.0.0.1:19222) in your browser, open **Capsem**
(in Applications on macOS), or choose **Dashboard** from the Capsem system tray menu. Click `my-work` in the list of VMs to open it,
then use its **Terminal** view to work inside the VM. The same VM tab also
lets you inspect files, logs, and activity.

## Pause your work and return later

Use **Stop** in Capsem or `Alt+T` in the TUI to free a persistent session's CPU
and memory. Files and setup survive; running programs restart. Warm suspend is
disabled in 0.7.1 pending [#378](https://github.com/google/capsem/issues/378).
When you return, start the session and open its terminal:

```sh
capsem resume my-work
capsem shell my-work
```

After resuming, you can also return through the [Capsem UI](#from-the-capsem-ui).

## Fork your VM

After setting up your tools and files in `my-work`, you can reuse that VM's
setup instead of doing it again. `capsem fork` duplicates your VM's files and setup
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
changing `my-work`. Programs start afresh in the new VM.

## Choose an image for your VM

`--image` accepts a Capsem prebuilt image or an authorized custom OCI image.
Use `capsem images` to see the available catalog. For other agents, custom
images, temporary VMs, and CPU/memory choices, see
[Configure and reuse your environment](/guides/users/reuse-your-environment/).

## Next steps

Once you are comfortable creating and using VMs, choose the next path for
your task:

- [Guides](/guides/) for task-based how-tos.
- [Reference](/reference/) for the complete CLI and MCP contracts.
- [SDK](/sdk/) to manage VMs from your application.
