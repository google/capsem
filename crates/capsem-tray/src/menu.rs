use muda::{Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};

use crate::gateway::{StatusResponse, UpdateStatusResponse, VmSummary};

/// Actions the tray can trigger, parsed from menu item IDs.
#[derive(Debug, PartialEq)]
pub enum Action {
    Connect(String),
    Stop(String),
    Delete(String),
    Suspend(String),
    Resume(String),
    /// Convert an ephemeral VM to persistent. Delegated to the UI because
    /// a tray menu can't prompt for the new name.
    Save(String),
    /// Fork a persistent VM into a new image. Delegated to the UI for
    /// the same reason.
    Fork(String),
    NewSession,
    OpenUi,
    StartService,
    Quit,
}

// -- Testable menu spec layer (no macOS main thread requirement) --

/// A menu entry description, independent of the muda toolkit.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MenuEntry {
    Item { id: String, label: String, enabled: bool },
    Separator,
    Sub { label: String, items: Vec<MenuEntry> },
}

/// Compute the menu structure for a gateway status response.
pub(crate) fn menu_spec(status: &StatusResponse) -> Vec<MenuEntry> {
    if !service_available(status) {
        return unavailable_spec();
    }

    let mut entries = Vec::new();

    // Status line at the top
    entries.push(MenuEntry::Item {
        id: "status".into(),
        label: format!("Connected -- {}ms", status.latency_ms.unwrap_or(0)),
        enabled: false,
    });
    if let Some(entry) = update_menu_entry(status) {
        entries.push(entry);
    }
    entries.push(MenuEntry::Separator);

    if !status.vms.is_empty() {
        entries.push(MenuEntry::Item {
            id: "header-sessions".into(),
            label: "Sessions".into(),
            enabled: false,
        });
        for vm in &status.vms {
            entries.push(vm_submenu_spec(vm));
        }
    }

    entries.push(MenuEntry::Separator);
    entries.push(MenuEntry::Item {
        id: "new-session".into(),
        label: "New Session".into(),
        enabled: true,
    });
    entries.push(MenuEntry::Item {
        id: "open".into(),
        label: "Dashboard".into(),
        enabled: true,
    });
    entries.push(MenuEntry::Separator);
    entries.push(MenuEntry::Item {
        id: "quit".into(),
        label: "Quit".into(),
        enabled: true,
    });

    entries
}

fn vm_submenu_spec(vm: &VmSummary) -> MenuEntry {
    let label = vm_label(vm);
    let id = &vm.id;
    // Gateway serializes VmState via its Display impl (capitalized:
    // "Running", "Suspended", "Stopped"), but older data and tests may
    // use lowercase -- compare case-insensitively.
    let status = vm.status.to_ascii_lowercase();
    let is_running = status == "running";
    let is_suspended = status == "suspended";
    let mut items = Vec::new();

    // Reachability: "Connect" for running, "Resume" for suspended, nothing
    // otherwise (stopped persistent VMs need an explicit resume).
    if is_running {
        items.push(MenuEntry::Item {
            id: format!("connect:{id}"),
            label: "Connect".into(),
            enabled: true,
        });
    } else if is_suspended {
        items.push(MenuEntry::Item {
            id: format!("resume:{id}"),
            label: "Resume".into(),
            enabled: true,
        });
    }

    if vm.persistent {
        // Persistent VMs: stop preserves state (only when running); delete
        // is destructive; fork clones to an image.
        if is_running {
            items.push(MenuEntry::Item {
                id: format!("stop:{id}"),
                label: "Stop".into(),
                enabled: true,
            });
        }
        items.push(MenuEntry::Item {
            id: format!("fork:{id}"),
            label: "Fork...".into(),
            enabled: true,
        });
        items.push(MenuEntry::Item {
            id: format!("delete:{id}"),
            label: "Delete".into(),
            enabled: true,
        });
    } else {
        // Ephemeral VMs: no "stop" (stopping == destruction). Save converts
        // to persistent; delete destroys.
        if is_running {
            items.push(MenuEntry::Item {
                id: format!("save:{id}"),
                label: "Save...".into(),
                enabled: true,
            });
        }
        items.push(MenuEntry::Item {
            id: format!("delete:{id}"),
            label: "Delete".into(),
            enabled: true,
        });
    }

    MenuEntry::Sub { label, items }
}

fn update_menu_entry(status: &StatusResponse) -> Option<MenuEntry> {
    let label = if let Some(updates) = &status.updates {
        update_status_label(updates)?
    } else if status.update_error.is_some() {
        "Updates: unavailable".into()
    } else {
        return None;
    };

    Some(MenuEntry::Item {
        id: "updates".into(),
        label,
        enabled: false,
    })
}

fn update_status_label(updates: &UpdateStatusResponse) -> Option<String> {
    let mut labels = Vec::new();
    if updates.binary.update_available {
        labels.push("Binary");
    }
    if updates.assets.update_available {
        labels.push("VM assets");
    }
    if updates.profiles.update_available {
        labels.push("Profiles");
    }
    if updates.images.update_available {
        labels.push("Images");
    }

    let mut blocked = Vec::new();
    if updates.binary.blocked_reason.is_some() {
        blocked.push("Binary");
    }
    if updates.assets.blocked_reason.is_some() {
        blocked.push("VM assets");
    }
    if updates.profiles.blocked_reason.is_some() {
        blocked.push("Profiles");
    }
    if updates.images.blocked_reason.is_some() {
        blocked.push("Images");
    }

    if !labels.is_empty() && !blocked.is_empty() {
        return Some(format!(
            "Updates: {}; blocked: {}",
            labels.join(", "),
            blocked.join(", ")
        ));
    }
    if !labels.is_empty() {
        return Some(format!("Updates: {}", labels.join(", ")));
    }
    if !blocked.is_empty() {
        return Some(format!("Updates blocked: {}", blocked.join(", ")));
    }
    if updates.last_error.is_some() {
        return Some("Updates: unavailable".into());
    }
    if updates.stale {
        return Some("Updates: check stale".into());
    }
    None
}

pub(crate) fn unavailable_spec() -> Vec<MenuEntry> {
    vec![
        MenuEntry::Item {
            id: "status".into(),
            label: "Disconnected".into(),
            enabled: false,
        },
        MenuEntry::Separator,
        MenuEntry::Item {
            id: "start-service".into(),
            label: "Start Service".into(),
            enabled: true,
        },
        MenuEntry::Separator,
        MenuEntry::Item {
            id: "quit".into(),
            label: "Quit".into(),
            enabled: true,
        },
    ]
}

pub(crate) fn service_available(status: &StatusResponse) -> bool {
    status.service.eq_ignore_ascii_case("running")
}

// -- muda rendering (requires macOS main thread) --

fn render_menu(spec: &[MenuEntry]) -> Menu {
    let menu = Menu::new();
    for entry in spec {
        match entry {
            MenuEntry::Item { id, label, enabled } => {
                menu.append(&MenuItem::with_id(MenuId::new(id), label, *enabled, None))
                    .unwrap();
            }
            MenuEntry::Separator => {
                menu.append(&PredefinedMenuItem::separator()).unwrap();
            }
            MenuEntry::Sub { label, items } => {
                let submenu = Submenu::new(label, true);
                for child in items {
                    if let MenuEntry::Item { id, label, enabled } = child {
                        submenu
                            .append(&MenuItem::with_id(MenuId::new(id), label, *enabled, None))
                            .unwrap();
                    }
                }
                menu.append(&submenu).unwrap();
            }
        }
    }
    menu
}

/// Build the full tray menu from a gateway status response.
pub fn build_menu(status: &StatusResponse) -> Menu {
    render_menu(&menu_spec(status))
}

/// Build a minimal menu for when the gateway is unreachable.
pub fn build_unavailable_menu() -> Menu {
    render_menu(&unavailable_spec())
}

/// Parse a MenuId string into an Action.
pub fn parse_action(id: &MenuId) -> Option<Action> {
    let s = id.as_ref();
    if let Some(vm_id) = s.strip_prefix("connect:") {
        return Some(Action::Connect(vm_id.to_string()));
    }
    if let Some(vm_id) = s.strip_prefix("stop:") {
        return Some(Action::Stop(vm_id.to_string()));
    }
    if let Some(vm_id) = s.strip_prefix("delete:") {
        return Some(Action::Delete(vm_id.to_string()));
    }
    if let Some(vm_id) = s.strip_prefix("suspend:") {
        return Some(Action::Suspend(vm_id.to_string()));
    }
    if let Some(vm_id) = s.strip_prefix("resume:") {
        return Some(Action::Resume(vm_id.to_string()));
    }
    if let Some(vm_id) = s.strip_prefix("save:") {
        return Some(Action::Save(vm_id.to_string()));
    }
    if let Some(vm_id) = s.strip_prefix("fork:") {
        return Some(Action::Fork(vm_id.to_string()));
    }
    match s {
        "new-session" => Some(Action::NewSession),
        "open" => Some(Action::OpenUi),
        "start-service" => Some(Action::StartService),
        "quit" => Some(Action::Quit),
        _ => None,
    }
}

/// Display label for a VM in the menu.
pub(crate) fn vm_label(vm: &VmSummary) -> String {
    let display = vm.name.as_deref().unwrap_or(&vm.id);
    format!("{display} -- {}", vm.status)
}

#[cfg(test)]
mod tests;
