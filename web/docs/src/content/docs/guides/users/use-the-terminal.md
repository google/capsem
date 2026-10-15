---
title: Use Capsem from the terminal
description: How do I open a VM terminal, run commands, and transfer files from my computer?
---

The terminal interface (TUI) lets you work in several VMs from one terminal
window: switch between their terminals, check their status, and create, fork,
suspend, or resume VMs. Use `capsem exec` for a single command and `capsem cp`
for file transfers.

[Install Capsem](/getting-started/installation/) first, then
[create a VM](/getting-started/how-to-use-capsem/#launch-your-first-vm).
The examples below use a VM named `my-work`; replace it with your VM's name
or ID from `capsem list`.

## Open the TUI

On your computer, open the TUI with all your VMs available:

```sh
capsem shell
```

To start with a particular VM selected:

```sh
capsem shell my-work
```

The main area shows the selected VM's terminal. Type there to run commands
inside that VM; for a VM created from an image, commands run in its container.
The bottom bar shows numbered VM tabs, marks the selected VM, and displays
connection latency and usage information. With two VMs, the layout is:

```text
┌──────────────────────────────────────────────────────┐
│ Terminal of the selected VM                          │
│                                                      │
│ Type commands here; Alt + Left/Right changes the VM.  │
│                                                      │
├──────────────────────────────────────────────────────┤
│ connection   1 my-work | 2 experiment   usage   help   │
└──────────────────────────────────────────────────────┘
```

`my-work` and `experiment` are example VM names. The bottom bar highlights
whichever VM you are working in.

## Work with multiple VMs

Keep the TUI open while switching between VMs. Each tab selects a VM's terminal;
switching tabs lets you follow several agents or tasks without opening a
separate terminal window for each VM.

| Key | What you can do |
| --- | --- |
| **Alt + Left / Right** | Switch to the previous or next VM tab. |
| **Alt + 1–9** | Jump directly to a numbered VM tab. |
| **Alt + L** | Show the VM list with names, status, and usage. |
| **Alt + I** | Inspect details for the selected VM. |
| **Alt + N** | Open the form to create another VM. |
| **Alt + F** | Open the form to fork the selected VM. |
| **Alt + S** | Suspend the selected persistent VM. |
| **Alt + R** | Resume the selected inactive VM. |
| **Alt + ?** | Show the full shortcut list. |
| **Esc** | Close a list, help panel, or form and return to the terminal. |
| **Alt + Q** | Close the TUI while leaving your VMs running. |

For example, press **Alt + N** to create another VM, then use
**Alt + Left / Right** to move between its terminal and your original VM.
Press **Alt + L** when you need to check which VMs are running or suspended.
To return after closing the TUI, run `capsem shell` again.

## Run one command

From a separate terminal on your computer:

```sh
capsem exec my-work 'pwd'
capsem exec my-work 'python3 --version' --timeout 30
```

The command runs in the image workload by default, or in the VM when it has
no workload. For VM-level diagnostics, add `--target vm`.
If a workload target is refused, Capsem does not silently switch to the VM.

The CLI writes stdout and stderr separately and returns the command's exit code.
Quote shell expressions so your computer's shell does not expand them first.

## Transfer files

Once the VM is running, upload a local file and download it again:

```sh
capsem cp ./task.txt my-work:task.txt
capsem cp my-work:task.txt ./task-copy.txt
```

Create `task.txt` on your computer first. The VM-side paths are relative to
its workspace. Compare `task-copy.txt` with your original to check the transfer.
Exactly one side of `capsem cp` must name a VM.

## Pause or reuse work

When you want to free CPU and memory without losing running programs:

```sh
capsem suspend my-work
capsem resume my-work
```

Suspension requires a persistent VM. For duplication, image selection, and
temporary tasks, see [Configure and reuse your environment](/guides/users/reuse-your-environment/).
The [CLI reference](/reference/cli/) lists exact options.
