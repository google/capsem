---
title: Use Capsem from the terminal
description: How do I open a VM terminal, run commands, and transfer files from my computer?
---

Use the TUI for an interactive terminal, `capsem exec` for one command,
and `capsem cp` for files. You need a running VM; this guide uses
`my-work`. Replace it with your VM's name or ID from `capsem list`.

## Open an interactive terminal

On your computer:

```sh
capsem shell my-work
```

Once the TUI opens, type commands in its terminal to run them inside the VM.
For an image VM, the terminal is in its container workload.

Use **Alt + Left/Right** to switch VMs and **Alt + Q** to close the TUI.
Closing the TUI leaves the VM running. Reopen it with the same command.
To see every VM instead of focusing one, use `capsem shell`.

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
