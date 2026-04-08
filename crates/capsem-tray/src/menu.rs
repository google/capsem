use muda::{Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};

use crate::gateway::{StatusResponse, VmSummary};

/// Actions the tray can trigger, parsed from menu item IDs.
#[derive(Debug)]
pub enum Action {
    Connect(String),
    Stop(String),
    Delete(String),
    Suspend(String),
    Resume(String),
    Fork(String),
    NewTemp,
    NewNamed,
    OpenUi,
    Quit,
}

/// Build the full tray menu from a gateway status response.
pub fn build_menu(status: &StatusResponse) -> Menu {
    let menu = Menu::new();

    // Per-VM submenus
    for vm in &status.vms {
        let label = vm_label(vm);
        let submenu = Submenu::new(&label, true);

        let id = &vm.id;
        submenu
            .append(&MenuItem::with_id(
                MenuId::new(format!("connect:{id}")),
                "Connect",
                true,
                None,
            ))
            .unwrap();

        // Suspend/Resume toggle based on current status
        if vm.status == "suspended" {
            submenu
                .append(&MenuItem::with_id(
                    MenuId::new(format!("resume:{id}")),
                    "Resume",
                    true,
                    None,
                ))
                .unwrap();
        } else {
            submenu
                .append(&MenuItem::with_id(
                    MenuId::new(format!("suspend:{id}")),
                    "Suspend",
                    true,
                    None,
                ))
                .unwrap();
        }

        submenu
            .append(&MenuItem::with_id(
                MenuId::new(format!("fork:{id}")),
                "Fork",
                true,
                None,
            ))
            .unwrap();
        submenu
            .append(&MenuItem::with_id(
                MenuId::new(format!("stop:{id}")),
                "Stop",
                true,
                None,
            ))
            .unwrap();
        submenu
            .append(&MenuItem::with_id(
                MenuId::new(format!("delete:{id}")),
                "Delete",
                true,
                None,
            ))
            .unwrap();

        menu.append(&submenu).unwrap();
    }

    // Separator + global actions
    menu.append(&PredefinedMenuItem::separator()).unwrap();
    menu.append(&MenuItem::with_id(
        MenuId::new("new-temp"),
        "New Temporary VM",
        true,
        None,
    ))
    .unwrap();
    menu.append(&MenuItem::with_id(
        MenuId::new("new-named"),
        "New Long-term VM",
        true,
        None,
    ))
    .unwrap();
    menu.append(&MenuItem::with_id(
        MenuId::new("open"),
        "Open Capsem",
        true,
        None,
    ))
    .unwrap();
    menu.append(&PredefinedMenuItem::separator()).unwrap();
    menu.append(&MenuItem::with_id(
        MenuId::new("quit"),
        "Quit",
        true,
        None,
    ))
    .unwrap();

    menu
}

/// Build a minimal menu for when the gateway is unreachable.
pub fn build_unavailable_menu() -> Menu {
    let menu = Menu::new();
    menu.append(&MenuItem::with_id(
        MenuId::new("unavailable"),
        "Service unavailable",
        false,
        None,
    ))
    .unwrap();
    menu.append(&PredefinedMenuItem::separator()).unwrap();
    menu.append(&MenuItem::with_id(
        MenuId::new("quit"),
        "Quit",
        true,
        None,
    ))
    .unwrap();
    menu
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
    if let Some(vm_id) = s.strip_prefix("fork:") {
        return Some(Action::Fork(vm_id.to_string()));
    }
    match s {
        "new-temp" => Some(Action::NewTemp),
        "new-named" => Some(Action::NewNamed),
        "open" => Some(Action::OpenUi),
        "quit" => Some(Action::Quit),
        _ => None,
    }
}

/// Display label for a VM in the menu.
fn vm_label(vm: &VmSummary) -> String {
    let display = vm
        .name
        .as_deref()
        .unwrap_or_else(|| &vm.id[..vm.id.len().min(8)]);
    format!("{display} -- {}", vm.status)
}
