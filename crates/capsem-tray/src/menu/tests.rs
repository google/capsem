use super::*;
use crate::gateway::{
    UpdateCompatibilityState, UpdateStatusResponse, UpdateTrackState, UpdateTrackStatus,
};
use muda::MenuId;

fn make_status(vms: Vec<VmSummary>) -> StatusResponse {
    let vm_count = vms.len() as u32;
    StatusResponse {
        service: "running".into(),
        vm_count,
        vms,
        latency_ms: Some(5),
        updates: None,
        update_error: None,
    }
}

fn make_service_status(service: &str, vms: Vec<VmSummary>) -> StatusResponse {
    let vm_count = vms.len() as u32;
    StatusResponse {
        service: service.into(),
        vm_count,
        vms,
        latency_ms: Some(5),
        updates: None,
        update_error: None,
    }
}

fn current_track() -> UpdateTrackStatus {
    UpdateTrackStatus {
        current: Some("1.4.0".into()),
        latest: Some("1.4.0".into()),
        blocked_reason: None,
        update_available: false,
        state: UpdateTrackState::Current,
        compatibility: UpdateCompatibilityState::Compatible,
    }
}

fn available_track(current: &str, latest: &str) -> UpdateTrackStatus {
    UpdateTrackStatus {
        current: Some(current.into()),
        latest: Some(latest.into()),
        blocked_reason: None,
        update_available: true,
        state: UpdateTrackState::UpdateAvailable,
        compatibility: UpdateCompatibilityState::Compatible,
    }
}

fn blocked_track(current: &str, latest: &str, reason: &str) -> UpdateTrackStatus {
    UpdateTrackStatus {
        current: Some(current.into()),
        latest: Some(latest.into()),
        blocked_reason: Some(reason.into()),
        update_available: false,
        state: UpdateTrackState::Current,
        compatibility: UpdateCompatibilityState::Unknown,
    }
}

fn update_status() -> UpdateStatusResponse {
    UpdateStatusResponse {
        checked_at: Some(1_718_444_400),
        channel_url: Some("https://release.capsem.org/health.json".into()),
        stale: false,
        last_error: None,
        binary: current_track(),
        assets: current_track(),
        profiles: current_track(),
        images: current_track(),
    }
}

fn with_updates(mut status: StatusResponse, updates: UpdateStatusResponse) -> StatusResponse {
    status.updates = Some(updates);
    status
}

fn with_update_error(mut status: StatusResponse, error: &str) -> StatusResponse {
    status.update_error = Some(error.into());
    status
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
            .filter_map(|e| {
                if let MenuEntry::Item { id, .. } = e {
                    Some(id.clone())
                } else {
                    None
                }
            })
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
    assert_eq!(
        parse_action(&MenuId::new("stop:vm-99")),
        Some(Action::Stop("vm-99".into()))
    );
}

#[test]
fn parse_delete() {
    assert_eq!(
        parse_action(&MenuId::new("delete:xyz")),
        Some(Action::Delete("xyz".into()))
    );
}

#[test]
fn parse_suspend() {
    assert_eq!(
        parse_action(&MenuId::new("suspend:s1")),
        Some(Action::Suspend("s1".into()))
    );
}

#[test]
fn parse_resume() {
    assert_eq!(
        parse_action(&MenuId::new("resume:s1")),
        Some(Action::Resume("s1".into()))
    );
}

#[test]
fn parse_new_session() {
    assert_eq!(
        parse_action(&MenuId::new("new-session")),
        Some(Action::NewSession)
    );
}

#[test]
fn parse_open() {
    assert_eq!(parse_action(&MenuId::new("open")), Some(Action::OpenUi));
}

#[test]
fn parse_start_service() {
    assert_eq!(
        parse_action(&MenuId::new("start-service")),
        Some(Action::StartService)
    );
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
    assert_eq!(
        vm_label(&named_vm("abc123def456", "dev", "running")),
        "dev -- running"
    );
}

#[test]
fn label_unnamed_vm_shows_full_id() {
    assert_eq!(
        vm_label(&temp_vm("abc123def456", "running")),
        "abc123def456 -- running"
    );
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
    assert_eq!(
        vm_label(&named_vm("deadbeef1234", "test-env", "suspended")),
        "test-env -- suspended"
    );
}

// -- menu_spec structure --

#[test]
fn spec_empty_has_global_actions_only() {
    let spec = menu_spec(&make_status(vec![]));
    let ids = collect_ids(&spec);
    assert!(!ids.contains(&"header-sessions".into()));
    assert!(!ids.contains(&"updates".into()));
    assert!(ids.contains(&"new-session".into()));
    assert!(ids.contains(&"open".into()));
    assert!(ids.contains(&"quit".into()));
}

#[test]
fn spec_current_updates_stays_quiet() {
    let spec = menu_spec(&with_updates(make_status(vec![]), update_status()));
    let ids = collect_ids(&spec);
    assert!(!ids.contains(&"updates".into()));
}

#[test]
fn spec_binary_update_shows_update_indicator() {
    let mut updates = update_status();
    updates.binary = available_track("1.4.0", "1.4.1");

    let spec = menu_spec(&with_updates(make_status(vec![]), updates));

    assert!(spec.iter().any(|entry| matches!(
        entry,
        MenuEntry::Item { id, label, enabled: false }
            if id == "updates" && label == "Updates: Binary"
    )));
}

#[test]
fn spec_asset_profile_and_image_updates_share_indicator() {
    let mut updates = update_status();
    updates.assets = available_track("assets-1", "assets-2");
    updates.profiles = available_track("profiles-1", "profiles-2");
    updates.images = available_track("images-1", "images-2");

    let spec = menu_spec(&with_updates(make_status(vec![]), updates));

    assert!(spec.iter().any(|entry| matches!(
        entry,
        MenuEntry::Item { id, label, enabled: false }
            if id == "updates" && label == "Updates: VM assets, Profiles, Images"
    )));
}

#[test]
fn spec_mixed_binary_and_asset_updates_share_indicator() {
    let mut updates = update_status();
    updates.binary = available_track("1.4.0", "1.4.1");
    updates.assets = available_track("2026.0627.1", "2030.0101.1");

    let spec = menu_spec(&with_updates(make_status(vec![]), updates));

    assert!(spec.iter().any(|entry| matches!(
        entry,
        MenuEntry::Item { id, label, enabled: false }
            if id == "updates" && label == "Updates: Binary, VM assets"
    )));
}

#[test]
fn spec_blocked_profile_update_shows_blocked_indicator() {
    let mut updates = update_status();
    updates.profiles = blocked_track(
        "profiles-2030.0101.0",
        "profiles-2030.0101.1",
        "requires binary 1.4.1 or newer",
    );

    let spec = menu_spec(&with_updates(make_status(vec![]), updates));

    assert!(spec.iter().any(|entry| matches!(
        entry,
        MenuEntry::Item { id, label, enabled: false }
            if id == "updates" && label == "Updates blocked: Profiles"
    )));
}

#[test]
fn spec_blocked_asset_update_shows_blocked_indicator() {
    let mut updates = update_status();
    updates.assets = blocked_track(
        "2026.0627.1",
        "2030.0101.1",
        "requires binary 99.99.99 or newer",
    );

    let spec = menu_spec(&with_updates(make_status(vec![]), updates));

    assert!(spec.iter().any(|entry| matches!(
        entry,
        MenuEntry::Item { id, label, enabled: false }
            if id == "updates" && label == "Updates blocked: VM assets"
    )));
}

#[test]
fn spec_binary_update_keeps_blocked_profile_visible() {
    let mut updates = update_status();
    updates.binary = available_track("1.4.0", "1.4.1");
    updates.profiles = blocked_track(
        "profiles-2030.0101.0",
        "profiles-2030.0101.1",
        "requires binary 1.4.1 or newer",
    );

    let spec = menu_spec(&with_updates(make_status(vec![]), updates));

    assert!(spec.iter().any(|entry| matches!(
        entry,
        MenuEntry::Item { id, label, enabled: false }
            if id == "updates" && label == "Updates: Binary; blocked: Profiles"
    )));
}

#[test]
fn spec_update_fetch_error_shows_unavailable_indicator() {
    let spec = menu_spec(&with_update_error(
        make_status(vec![]),
        "gateway returned 404",
    ));

    assert!(spec.iter().any(|entry| matches!(
        entry,
        MenuEntry::Item { id, label, enabled: false }
            if id == "updates" && label == "Updates: unavailable"
    )));
}

#[test]
fn spec_non_running_service_does_not_offer_dead_session_actions() {
    let spec = menu_spec(&make_service_status(
        "stopped",
        vec![named_vm("n1", "dev", "running")],
    ));
    let ids = collect_ids(&spec);

    assert_eq!(ids, vec!["status", "start-service", "quit"]);
    assert!(!ids.contains(&"header-sessions".into()));
    assert!(!ids.contains(&"connect:n1".into()));
    assert!(!ids.contains(&"new-session".into()));
    assert!(!ids.contains(&"open".into()));
    assert!(
        matches!(&spec[0], MenuEntry::Item { label, enabled: false, .. } if label == "Disconnected")
    );
}

#[test]
fn spec_with_vms_shows_sessions_header() {
    let spec = menu_spec(&make_status(vec![temp_vm("vm1", "running")]));
    let ids = collect_ids(&spec);
    assert!(ids.contains(&"header-sessions".into()));
}

#[test]
fn spec_sessions_header_is_disabled() {
    let spec = menu_spec(&make_status(vec![temp_vm("vm1", "running")]));
    let hdr = spec
        .iter()
        .find(|e| matches!(e, MenuEntry::Item { id, .. } if id == "header-sessions"))
        .unwrap();
    assert!(matches!(hdr, MenuEntry::Item { enabled: false, .. }));
}

#[test]
fn persistent_running_vm_has_connect_stop_fork_delete() {
    let spec = menu_spec(&make_status(vec![named_vm("n1", "prod", "running")]));
    let sub = spec
        .iter()
        .find(|e| matches!(e, MenuEntry::Sub { .. }))
        .unwrap();
    let ids = submenu_child_ids(sub);
    assert_eq!(ids, vec!["connect:n1", "stop:n1", "fork:n1", "delete:n1"]);
}

#[test]
fn ephemeral_running_vm_has_connect_save_delete() {
    // Ephemeral VMs cannot be stopped (stopping == destruction).
    // Save converts to persistent; delete destroys.
    let spec = menu_spec(&make_status(vec![temp_vm("t1", "running")]));
    let sub = spec
        .iter()
        .find(|e| matches!(e, MenuEntry::Sub { .. }))
        .unwrap();
    let ids = submenu_child_ids(sub);
    assert_eq!(ids, vec!["connect:t1", "save:t1", "delete:t1"]);
}

#[test]
fn persistent_suspended_vm_has_resume_fork_delete() {
    // Suspended persistent VMs have no "stop" (already not running).
    let spec = menu_spec(&make_status(vec![named_vm("s1", "staging", "suspended")]));
    let sub = spec
        .iter()
        .find(|e| matches!(e, MenuEntry::Sub { .. }))
        .unwrap();
    let ids = submenu_child_ids(sub);
    assert_eq!(ids, vec!["resume:s1", "fork:s1", "delete:s1"]);
}

#[test]
fn ephemeral_suspended_vm_has_resume_delete() {
    // Edge case: ephemeral VM in a suspended state. No save (ephemeral
    // must be running to save), no fork (fork is persistent-only).
    let spec = menu_spec(&make_status(vec![temp_vm("t2", "suspended")]));
    let sub = spec
        .iter()
        .find(|e| matches!(e, MenuEntry::Sub { .. }))
        .unwrap();
    let ids = submenu_child_ids(sub);
    assert_eq!(ids, vec!["resume:t2", "delete:t2"]);
}

#[test]
fn persistent_stopped_vm_has_fork_delete_no_connect() {
    // Stopped persistent (not suspended) -- needs explicit resume from
    // the UI dialog, but fork and delete remain.
    let spec = menu_spec(&make_status(vec![named_vm("s2", "prod", "stopped")]));
    let sub = spec
        .iter()
        .find(|e| matches!(e, MenuEntry::Sub { .. }))
        .unwrap();
    let ids = submenu_child_ids(sub);
    assert_eq!(ids, vec!["fork:s2", "delete:s2"]);
}

#[test]
fn unavailable_spec_has_disconnected_and_quit() {
    let spec = unavailable_spec();
    let ids = collect_ids(&spec);
    assert_eq!(ids, vec!["status", "start-service", "quit"]);
    // status shows "Disconnected" and is disabled
    assert!(
        matches!(&spec[0], MenuEntry::Item { label, enabled: false, .. } if label == "Disconnected")
    );
}

#[test]
fn spec_many_vms_preserves_order() {
    let spec = menu_spec(&make_status(vec![
        temp_vm("t1", "running"),
        named_vm("n1", "dev", "running"),
        temp_vm("t2", "suspended"),
        named_vm("n2", "prod", "running"),
    ]));
    let ids = collect_ids(&spec);
    let t1_pos = ids.iter().position(|id| id.contains("t1")).unwrap();
    let n1_pos = ids.iter().position(|id| id.contains("n1")).unwrap();
    let t2_pos = ids.iter().position(|id| id.contains("t2")).unwrap();
    let n2_pos = ids.iter().position(|id| id.contains("n2")).unwrap();
    assert!(t1_pos < n1_pos);
    assert!(n1_pos < t2_pos);
    assert!(t2_pos < n2_pos);
}

#[test]
fn spec_global_actions_always_present() {
    for vms in [
        vec![],
        vec![named_vm("n", "x", "running")],
        vec![temp_vm("t", "running")],
    ] {
        let spec = menu_spec(&make_status(vms));
        let ids = collect_ids(&spec);
        assert!(ids.contains(&"new-session".into()));
        assert!(ids.contains(&"open".into()));
        assert!(ids.contains(&"quit".into()));
    }
}

#[test]
fn spec_sessions_header_disabled_with_mixed_vms() {
    let spec = menu_spec(&make_status(vec![
        named_vm("n1", "dev", "running"),
        temp_vm("t1", "running"),
    ]));
    let hdr = spec
        .iter()
        .find(|e| matches!(e, MenuEntry::Item { id, .. } if id == "header-sessions"))
        .unwrap();
    assert!(matches!(hdr, MenuEntry::Item { enabled: false, .. }));
}

// ── Status casing from a real gateway ──────────────────────────────
//
// The gateway serializes VmState through its Display impl, which is
// capitalized: "Running", "Suspended", "Stopped". Every test above uses the
// lowercase form, so the shape the tray actually receives in production was
// the one shape nothing exercised. Both comparison sites fold case on purpose;
// if either regressed to `==`, a live service would render as unavailable and
// every running VM would lose its Connect entry.

#[test]
fn service_availability_folds_case_as_the_gateway_capitalizes_it() {
    for form in ["Running", "running", "RUNNING", "RuNnInG"] {
        assert!(
            service_available(&make_service_status(form, vec![])),
            "{form:?} is a live service"
        );
    }
}

#[test]
fn service_availability_rejects_every_other_state() {
    for form in ["Stopped", "stopped", "starting", "", "run", "running "] {
        assert!(
            !service_available(&make_service_status(form, vec![])),
            "{form:?} must not read as available"
        );
    }
}

#[test]
fn a_capitalized_running_vm_still_offers_connect() {
    let spec = menu_spec(&make_status(vec![named_vm("vm1", "dev", "Running")]));

    assert!(
        collect_ids(&spec).iter().any(|id| id == "connect:vm1"),
        "the gateway's own casing must not hide Connect: {:?}",
        collect_ids(&spec)
    );
}

#[test]
fn a_capitalized_suspended_vm_still_offers_resume() {
    let spec = menu_spec(&make_status(vec![named_vm("vm2", "dev", "Suspended")]));

    assert!(
        collect_ids(&spec).iter().any(|id| id == "resume:vm2"),
        "expected a resume entry, got {:?}",
        collect_ids(&spec)
    );
}

#[test]
fn an_unrecognised_status_offers_no_reachability_action() {
    // A future or garbled state must not be guessed into Connect, which would
    // dial a VM that is not there.
    let spec = menu_spec(&make_status(vec![named_vm("vm3", "dev", "Provisioning")]));
    let ids = collect_ids(&spec);

    assert!(!ids.iter().any(|id| id == "connect:vm3"), "{ids:?}");
    assert!(!ids.iter().any(|id| id == "resume:vm3"), "{ids:?}");
}
