use muda::{Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};

use crate::gateway::{StatusResponse, VmSummary};

/// Actions the tray can trigger, parsed from menu item IDs.
#[derive(Debug, PartialEq)]
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
pub(crate) fn vm_label(vm: &VmSummary) -> String {
    let display = vm
        .name
        .as_deref()
        .unwrap_or_else(|| &vm.id[..vm.id.len().min(8)]);
    format!("{display} -- {}", vm.status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use muda::MenuId;

    // -- parse_action --

    #[test]
    fn parse_connect() {
        let id = MenuId::new("connect:abc123");
        assert_eq!(
            parse_action(&id),
            Some(Action::Connect("abc123".into()))
        );
    }

    #[test]
    fn parse_stop() {
        let id = MenuId::new("stop:vm-99");
        assert_eq!(parse_action(&id), Some(Action::Stop("vm-99".into())));
    }

    #[test]
    fn parse_delete() {
        let id = MenuId::new("delete:xyz");
        assert_eq!(parse_action(&id), Some(Action::Delete("xyz".into())));
    }

    #[test]
    fn parse_suspend() {
        let id = MenuId::new("suspend:s1");
        assert_eq!(parse_action(&id), Some(Action::Suspend("s1".into())));
    }

    #[test]
    fn parse_resume() {
        let id = MenuId::new("resume:s1");
        assert_eq!(parse_action(&id), Some(Action::Resume("s1".into())));
    }

    #[test]
    fn parse_fork() {
        let id = MenuId::new("fork:vm-42");
        assert_eq!(parse_action(&id), Some(Action::Fork("vm-42".into())));
    }

    #[test]
    fn parse_new_temp() {
        assert_eq!(parse_action(&MenuId::new("new-temp")), Some(Action::NewTemp));
    }

    #[test]
    fn parse_new_named() {
        assert_eq!(
            parse_action(&MenuId::new("new-named")),
            Some(Action::NewNamed)
        );
    }

    #[test]
    fn parse_open() {
        assert_eq!(parse_action(&MenuId::new("open")), Some(Action::OpenUi));
    }

    #[test]
    fn parse_quit() {
        assert_eq!(parse_action(&MenuId::new("quit")), Some(Action::Quit));
    }

    #[test]
    fn parse_unknown_returns_none() {
        assert_eq!(parse_action(&MenuId::new("bogus")), None);
        assert_eq!(parse_action(&MenuId::new("")), None);
        assert_eq!(parse_action(&MenuId::new("unavailable")), None);
    }

    #[test]
    fn parse_action_with_colon_in_vm_id() {
        let id = MenuId::new("connect:vm:with:colons");
        assert_eq!(
            parse_action(&id),
            Some(Action::Connect("vm:with:colons".into()))
        );
    }

    // -- vm_label --

    #[test]
    fn label_named_vm() {
        let vm = VmSummary {
            id: "abc123def456".into(),
            name: Some("dev".into()),
            status: "running".into(),
            persistent: true,
        };
        assert_eq!(vm_label(&vm), "dev -- running");
    }

    #[test]
    fn label_unnamed_vm_truncates_id() {
        let vm = VmSummary {
            id: "abc123def456".into(),
            name: None,
            status: "running".into(),
            persistent: false,
        };
        assert_eq!(vm_label(&vm), "abc123de -- running");
    }

    #[test]
    fn label_short_unnamed_id() {
        let vm = VmSummary {
            id: "ab".into(),
            name: None,
            status: "stopped".into(),
            persistent: false,
        };
        assert_eq!(vm_label(&vm), "ab -- stopped");
    }

    #[test]
    fn label_suspended_vm() {
        let vm = VmSummary {
            id: "deadbeef1234".into(),
            name: Some("test-env".into()),
            status: "suspended".into(),
            persistent: true,
        };
        assert_eq!(vm_label(&vm), "test-env -- suspended");
    }
}
