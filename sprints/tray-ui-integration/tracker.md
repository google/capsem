# Tray-UI Integration Sprint -- Making the Dashboard Work with the Tray

The tray is live and functional as a status monitor + VM control panel. Several menu items launch the Capsem UI app via `open -a Capsem`, but the UI doesn't handle the required CLI arguments yet. This sprint bridges that gap.

Crate: crates/capsem-app/ (Tauri)
Depends on: S12 (tray sprint, complete)

## What the Tray Sends

The tray launches the UI with `open -a Capsem [--args ...]`:

| Tray Menu Item | Command | Expected UI Behavior |
|----------------|---------|---------------------|
| **Dashboard** | `open -a Capsem` | Launch or focus the main window |
| **Connect** (per-VM) | `open -a Capsem --args --connect {vm_id}` | Open focused on that VM (terminal, logs, or status) |
| **New Permanent...** | `open -a Capsem --args --new-named` | Open with a "create named VM" dialog |
| **New Temporary** | Tray provisions via API, then `--connect {id}` | Same as Connect (VM already created) |

## Sub-sprints

### STU1: Accept CLI Arguments in Tauri

Status: Not started

- [ ] Parse `--connect <vm_id>` CLI argument on launch
- [ ] Parse `--new-named` CLI argument on launch
- [ ] If launched with no args: show main dashboard (existing behavior)
- [ ] If already running and launched again with args: bring window to front and navigate to the requested view (macOS single-instance behavior via `open -a`)

### STU2: Connect View

Status: Not started

- [ ] When `--connect <vm_id>` is received: navigate to the VM detail view
- [ ] Show VM status, terminal, logs for the specified VM
- [ ] If the VM doesn't exist or has been deleted: show an error state
- [ ] Deep-link works from both cold start and warm (already running) launch

### STU3: New Named VM Dialog

Status: Not started

- [ ] When `--new-named` is received: open the "create VM" dialog with name field focused
- [ ] Dialog submits `POST /provision` with the chosen name and `persistent: true`
- [ ] After creation: navigate to the new VM's detail view
- [ ] Cancel returns to the main dashboard

### STU4: Single-Instance Handling

Status: Not started

- [ ] macOS: `open -a Capsem` reuses the existing instance
- [ ] Tauri's `single_instance` plugin or NSApplication delegate to receive new URLs/args
- [ ] When new args arrive on an already-running instance: parse and navigate accordingly
- [ ] Test: launch with `--connect`, then launch again with `--new-named` -- second launch should navigate without creating a new window

## Tray Code Reference

```
crates/capsem-tray/src/main.rs
  launch_ui(vm_id: Option<&str>)      -- "open -a Capsem [--args --connect {id}]"
  launch_ui_new_named()                -- "open -a Capsem --args --new-named"
```

## Acceptance Criteria

- [ ] `open -a Capsem` launches or focuses the dashboard
- [ ] `open -a Capsem --args --connect <vm_id>` shows the VM detail view
- [ ] `open -a Capsem --args --new-named` shows the create dialog
- [ ] All three work from both cold start and when app is already running
- [ ] Tray "Dashboard" menu item opens the UI
- [ ] Tray "Connect" on a VM opens the UI focused on that VM
- [ ] Tray "New Permanent..." opens the UI with name dialog
