---
title: Configure and reuse your environment
description: How do I choose an image, retain my tools and files, and duplicate a VM?
---

This guide explains how to choose a VM's image and resources, decide whether
to keep it, and reuse its tools and files. It covers temporary VMs, suspension,
and independent copies of a prepared VM. The examples assume
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

## Start from a saved environment

In Capsem 0.7.3, open **Saved environments** in the desktop's new-session
view. Search by name, description, or image, then choose **Use** beside the
environment you want. Enter a session name, choose resources, and select
**Create**.

The dialog shows the revision it will boot. Refreshing the library keeps the
selected revision. Choose **Change environment** and select the newer revision
if you want to use it.

## Save a new environment or revise one

In **Saved environments**, choose **New environment** or **Edit** beside an
existing environment. Set the image, startup script, and configuration you
want. Use the **Secret bindings** section for credential binding IDs.

Select **Review changes**, check the settings, enter a reason, and select
**Save environment**. Running sessions keep their existing revisions. If the
saved revision changes before you save, reload it before starting a new edit.

Open **Plugins** to inspect the settings that Capsem will use when this
environment starts. The editor shows the effective modes for each supported
direction and locks entries owned by corporate policy. Expand a plugin to
inspect its options, protocol coverage, and any saved values overridden by
corporate policy. Editable options are included in the same review before
saving.

Open **Access rules** or **Detection rules** on a saved environment to see
the compiled rules under current corporate policy. Access rules appear in
enforcement order, with lower priorities first. Detection rules show the
severity they report. You can filter by rule name or source and stage a
custom rule's enabled state before reviewing and saving.

To edit a saved revision as text, open **TOML** in its editor. Keep the
revision identity fields as shown. When changing an image, repository, or
catalog selector, remove its old resolution fields so Capsem can resolve the
new selection. **Review changes** validates the text against the selected
revision before you save.

Open **Change history** to inspect older revisions. Complete revisions show
canonical TOML. When values have been purged, the view identifies the removed
variables and shows the remaining content when it is available.

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
