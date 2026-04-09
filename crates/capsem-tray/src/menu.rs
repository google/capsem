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
    let mut entries = Vec::new();
    let named: Vec<&VmSummary> = status.vms.iter().filter(|v| v.persistent).collect();
    let ephemeral: Vec<&VmSummary> = status.vms.iter().filter(|v| !v.persistent).collect();

    if !named.is_empty() {
        entries.push(MenuEntry::Item {
            id: "header-named".into(),
            label: "Permanent".into(),
            enabled: false,
        });
        for vm in &named {
            entries.push(vm_submenu_spec(vm));
        }
    }

    if !ephemeral.is_empty() {
        if !named.is_empty() {
            entries.push(MenuEntry::Separator);
        }
        entries.push(MenuEntry::Item {
            id: "header-ephemeral".into(),
            label: "Temporary".into(),
            enabled: false,
        });
        for vm in &ephemeral {
            entries.push(vm_submenu_spec(vm));
        }
    }

    entries.push(MenuEntry::Separator);
    entries.push(MenuEntry::Item { id: "new-temp".into(), label: "New Temporary".into(), enabled: true });
    entries.push(MenuEntry::Item { id: "new-named".into(), label: "New Permanent...".into(), enabled: true });
    entries.push(MenuEntry::Item { id: "open".into(), label: "Open Capsem".into(), enabled: true });
    entries.push(MenuEntry::Separator);
    entries.push(MenuEntry::Item { id: "quit".into(), label: "Quit".into(), enabled: true });

    entries
}

fn vm_submenu_spec(vm: &VmSummary) -> MenuEntry {
    let label = vm_label(vm);
    let id = &vm.id;
    let mut items = Vec::new();

    if vm.status == "suspended" {
        items.push(MenuEntry::Item { id: format!("resume:{id}"), label: "Resume".into(), enabled: true });
    } else {
        items.push(MenuEntry::Item { id: format!("connect:{id}"), label: "Connect".into(), enabled: true });
    }

    if vm.persistent {
        items.push(MenuEntry::Item { id: format!("fork:{id}"), label: "Fork".into(), enabled: true });
        items.push(MenuEntry::Item { id: format!("stop:{id}"), label: "Stop".into(), enabled: true });
    }

    items.push(MenuEntry::Item { id: format!("delete:{id}"), label: "Delete".into(), enabled: true });

    MenuEntry::Sub { label, items }
}

pub(crate) fn unavailable_spec() -> Vec<MenuEntry> {
    vec![
        MenuEntry::Item { id: "unavailable".into(), label: "Service unavailable".into(), enabled: false },
        MenuEntry::Separator,
        MenuEntry::Item { id: "quit".into(), label: "Quit".into(), enabled: true },
    ]
}

// -- muda rendering (requires macOS main thread) --

fn render_menu(spec: &[MenuEntry]) -> Menu {
    let menu = Menu::new();
    for entry in spec {
        match entry {
            MenuEntry::Item { id, label, enabled } => {
                menu.append(&MenuItem::with_id(MenuId::new(id), label, *enabled, None)).unwrap();
            }
            MenuEntry::Separator => {
                menu.append(&PredefinedMenuItem::separator()).unwrap();
            }
            MenuEntry::Sub { label, items } => {
                let submenu = Submenu::new(label, true);
                for child in items {
                    if let MenuEntry::Item { id, label, enabled } = child {
                        submenu.append(&MenuItem::with_id(MenuId::new(id), label, *enabled, None)).unwrap();
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

    fn make_status(vms: Vec<VmSummary>) -> StatusResponse {
        let vm_count = vms.len() as u32;
        StatusResponse {
            service: "running".into(),
            vm_count,
            vms,
        }
    }

    fn named_vm(id: &str, name: &str, status: &str) -> VmSummary {
        VmSummary {
            id: id.into(),
            name: Some(name.into()),
            status: status.into(),
            persistent: true,
        }
    }

    fn temp_vm(id: &str, status: &str) -> VmSummary {
        VmSummary {
            id: id.into(),
            name: None,
            status: status.into(),
            persistent: false,
        }
    }

    /// Collect all item IDs from a spec, flattening submenus.
    fn collect_ids(spec: &[MenuEntry]) -> Vec<String> {
        let mut ids = Vec::new();
        for entry in spec {
            match entry {
                MenuEntry::Item { id, .. } => ids.push(id.clone()),
                MenuEntry::Sub { label, items } => {
                    ids.push(format!("submenu:{label}"));
                    for child in items {
                        if let MenuEntry::Item { id, .. } = child {
                            ids.push(id.clone());
                        }
                    }
                }
                MenuEntry::Separator => {}
            }
        }
        ids
    }

    /// Extract child IDs from a submenu entry.
    fn submenu_child_ids(entry: &MenuEntry) -> Vec<String> {
        match entry {
            MenuEntry::Sub { items, .. } => items
                .iter()
                .filter_map(|e| if let MenuEntry::Item { id, .. } = e { Some(id.clone()) } else { None })
                .collect(),
            _ => vec![],
        }
    }

    // -- parse_action --

    #[test]
    fn parse_connect() {
        assert_eq!(
            parse_action(&MenuId::new("connect:abc123")),
            Some(Action::Connect("abc123".into()))
        );
    }

    #[test]
    fn parse_stop() {
        assert_eq!(parse_action(&MenuId::new("stop:vm-99")), Some(Action::Stop("vm-99".into())));
    }

    #[test]
    fn parse_delete() {
        assert_eq!(parse_action(&MenuId::new("delete:xyz")), Some(Action::Delete("xyz".into())));
    }

    #[test]
    fn parse_suspend() {
        assert_eq!(parse_action(&MenuId::new("suspend:s1")), Some(Action::Suspend("s1".into())));
    }

    #[test]
    fn parse_resume() {
        assert_eq!(parse_action(&MenuId::new("resume:s1")), Some(Action::Resume("s1".into())));
    }

    #[test]
    fn parse_fork() {
        assert_eq!(parse_action(&MenuId::new("fork:vm-42")), Some(Action::Fork("vm-42".into())));
    }

    #[test]
    fn parse_new_temp() {
        assert_eq!(parse_action(&MenuId::new("new-temp")), Some(Action::NewTemp));
    }

    #[test]
    fn parse_new_named() {
        assert_eq!(parse_action(&MenuId::new("new-named")), Some(Action::NewNamed));
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
        assert_eq!(
            parse_action(&MenuId::new("connect:vm:with:colons")),
            Some(Action::Connect("vm:with:colons".into()))
        );
    }

    // -- vm_label --

    #[test]
    fn label_named_vm() {
        assert_eq!(vm_label(&named_vm("abc123def456", "dev", "running")), "dev -- running");
    }

    #[test]
    fn label_unnamed_vm_truncates_id() {
        assert_eq!(vm_label(&temp_vm("abc123def456", "running")), "abc123de -- running");
    }

    #[test]
    fn label_short_unnamed_id() {
        let vm = VmSummary { id: "ab".into(), name: None, status: "stopped".into(), persistent: false };
        assert_eq!(vm_label(&vm), "ab -- stopped");
    }

    #[test]
    fn label_suspended_vm() {
        assert_eq!(vm_label(&named_vm("deadbeef1234", "test-env", "suspended")), "test-env -- suspended");
    }

    // -- menu_spec structure --

    #[test]
    fn spec_empty_has_global_actions_only() {
        let spec = menu_spec(&make_status(vec![]));
        let ids = collect_ids(&spec);
        assert!(!ids.contains(&"header-named".into()));
        assert!(!ids.contains(&"header-ephemeral".into()));
        assert!(ids.contains(&"new-temp".into()));
        assert!(ids.contains(&"new-named".into()));
        assert!(ids.contains(&"open".into()));
        assert!(ids.contains(&"quit".into()));
    }

    #[test]
    fn spec_named_only_shows_permanent_header() {
        let spec = menu_spec(&make_status(vec![named_vm("vm1", "dev", "running")]));
        let ids = collect_ids(&spec);
        assert!(ids.contains(&"header-named".into()));
        assert!(!ids.contains(&"header-ephemeral".into()));
    }

    #[test]
    fn spec_temp_only_shows_temporary_header() {
        let spec = menu_spec(&make_status(vec![temp_vm("vm1", "running")]));
        let ids = collect_ids(&spec);
        assert!(!ids.contains(&"header-named".into()));
        assert!(ids.contains(&"header-ephemeral".into()));
    }

    #[test]
    fn spec_mixed_shows_both_headers() {
        let spec = menu_spec(&make_status(vec![
            named_vm("n1", "dev", "running"),
            temp_vm("t1", "running"),
        ]));
        let ids = collect_ids(&spec);
        assert!(ids.contains(&"header-named".into()));
        assert!(ids.contains(&"header-ephemeral".into()));
    }

    #[test]
    fn spec_permanent_header_before_temporary() {
        let spec = menu_spec(&make_status(vec![
            temp_vm("t1", "running"),
            named_vm("n1", "prod", "running"),
        ]));
        let ids = collect_ids(&spec);
        let named_pos = ids.iter().position(|id| id == "header-named").unwrap();
        let temp_pos = ids.iter().position(|id| id == "header-ephemeral").unwrap();
        assert!(named_pos < temp_pos);
    }

    #[test]
    fn spec_separator_between_sections() {
        let spec = menu_spec(&make_status(vec![
            named_vm("n1", "dev", "running"),
            temp_vm("t1", "running"),
        ]));
        // Find the separator between the two sections
        let named_idx = spec.iter().position(|e| matches!(e, MenuEntry::Item { id, .. } if id == "header-named")).unwrap();
        let temp_idx = spec.iter().position(|e| matches!(e, MenuEntry::Item { id, .. } if id == "header-ephemeral")).unwrap();
        let has_sep = spec[named_idx..temp_idx].iter().any(|e| matches!(e, MenuEntry::Separator));
        assert!(has_sep);
    }

    #[test]
    fn spec_no_separator_when_only_one_section() {
        let spec = menu_spec(&make_status(vec![named_vm("n1", "dev", "running")]));
        // Before the global separator, there should be no separator
        let global_sep = spec.iter().position(|e| matches!(e, MenuEntry::Item { id, .. } if id == "new-temp")).unwrap();
        let vm_seps = spec[..global_sep - 1].iter().filter(|e| matches!(e, MenuEntry::Separator)).count();
        assert_eq!(vm_seps, 0);
    }

    #[test]
    fn named_running_vm_has_connect_fork_stop_delete() {
        let spec = menu_spec(&make_status(vec![named_vm("n1", "prod", "running")]));
        let sub = spec.iter().find(|e| matches!(e, MenuEntry::Sub { .. })).unwrap();
        let ids = submenu_child_ids(sub);
        assert_eq!(ids, vec!["connect:n1", "fork:n1", "stop:n1", "delete:n1"]);
    }

    #[test]
    fn temp_running_vm_has_connect_delete_only() {
        let spec = menu_spec(&make_status(vec![temp_vm("t1", "running")]));
        let sub = spec.iter().find(|e| matches!(e, MenuEntry::Sub { .. })).unwrap();
        let ids = submenu_child_ids(sub);
        assert_eq!(ids, vec!["connect:t1", "delete:t1"]);
    }

    #[test]
    fn named_suspended_vm_has_resume_fork_stop_delete() {
        let spec = menu_spec(&make_status(vec![named_vm("s1", "staging", "suspended")]));
        let sub = spec.iter().find(|e| matches!(e, MenuEntry::Sub { .. })).unwrap();
        let ids = submenu_child_ids(sub);
        assert_eq!(ids, vec!["resume:s1", "fork:s1", "stop:s1", "delete:s1"]);
    }

    #[test]
    fn temp_suspended_vm_has_resume_delete_only() {
        let spec = menu_spec(&make_status(vec![temp_vm("t2", "suspended")]));
        let sub = spec.iter().find(|e| matches!(e, MenuEntry::Sub { .. })).unwrap();
        let ids = submenu_child_ids(sub);
        assert_eq!(ids, vec!["resume:t2", "delete:t2"]);
    }

    #[test]
    fn unavailable_spec_has_disabled_status_and_quit() {
        let spec = unavailable_spec();
        let ids = collect_ids(&spec);
        assert_eq!(ids, vec!["unavailable", "quit"]);
        // "unavailable" is disabled
        assert!(matches!(&spec[0], MenuEntry::Item { enabled: false, .. }));
    }

    #[test]
    fn spec_many_vms_named_before_temp() {
        let spec = menu_spec(&make_status(vec![
            temp_vm("t1", "running"),
            named_vm("n1", "dev", "running"),
            temp_vm("t2", "suspended"),
            named_vm("n2", "prod", "running"),
        ]));
        let ids = collect_ids(&spec);
        let n1_pos = ids.iter().position(|id| id.contains("n1")).unwrap();
        let n2_pos = ids.iter().position(|id| id.contains("n2")).unwrap();
        let t1_pos = ids.iter().position(|id| id.contains("t1")).unwrap();
        let t2_pos = ids.iter().position(|id| id.contains("t2")).unwrap();
        assert!(n1_pos < t1_pos);
        assert!(n2_pos < t1_pos);
        assert!(n1_pos < t2_pos);
    }

    #[test]
    fn spec_global_actions_always_present() {
        for vms in [vec![], vec![named_vm("n", "x", "running")], vec![temp_vm("t", "running")]] {
            let spec = menu_spec(&make_status(vms));
            let ids = collect_ids(&spec);
            assert!(ids.contains(&"new-temp".into()));
            assert!(ids.contains(&"new-named".into()));
            assert!(ids.contains(&"open".into()));
            assert!(ids.contains(&"quit".into()));
        }
    }

    #[test]
    fn spec_headers_are_disabled() {
        let spec = menu_spec(&make_status(vec![
            named_vm("n1", "dev", "running"),
            temp_vm("t1", "running"),
        ]));
        let named_hdr = spec.iter().find(|e| matches!(e, MenuEntry::Item { id, .. } if id == "header-named")).unwrap();
        let temp_hdr = spec.iter().find(|e| matches!(e, MenuEntry::Item { id, .. } if id == "header-ephemeral")).unwrap();
        assert!(matches!(named_hdr, MenuEntry::Item { enabled: false, .. }));
        assert!(matches!(temp_hdr, MenuEntry::Item { enabled: false, .. }));
    }
}
