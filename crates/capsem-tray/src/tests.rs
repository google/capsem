use super::*;
use std::ffi::OsString;
use std::path::PathBuf;

fn os(s: &str) -> OsString {
    OsString::from(s)
}

#[test]
fn direct_binary_no_vm_id_no_action() {
    let binary = PathBuf::from("/Applications/Capsem.app/Contents/MacOS/capsem-app");
    let (program, args) = build_launch_invocation(Some(&binary), None, None);
    assert_eq!(program, binary.as_os_str());
    assert!(args.is_empty(), "no deep-link args expected, got {args:?}");
}

#[test]
fn direct_binary_connects_to_vm() {
    let binary = PathBuf::from("/Applications/Capsem.app/Contents/MacOS/capsem-app");
    let (program, args) = build_launch_invocation(Some(&binary), Some("vm-123"), None);
    assert_eq!(program, binary.as_os_str());
    assert_eq!(args, vec![os("--connect"), os("vm-123")]);
}

#[test]
fn direct_binary_with_action_requires_vm_id() {
    let binary = PathBuf::from("/Applications/Capsem.app/Contents/MacOS/capsem-app");
    let (program, args) = build_launch_invocation(Some(&binary), Some("vm-42"), Some("save"));
    assert_eq!(program, binary.as_os_str());
    assert_eq!(
        args,
        vec![os("--connect"), os("vm-42"), os("--action"), os("save")]
    );
}

#[test]
fn fallback_open_no_args_when_no_deep_link() {
    // Without vm_id/action, `open -a Capsem` is enough -- no `--args`.
    // Appending `--args` with nothing after it would still work but is
    // unnecessary noise.
    let (program, args) = build_launch_invocation(None, None, None);
    assert_eq!(program, os("open"));
    assert_eq!(args, vec![os("-a"), os("Capsem")]);
}

#[test]
fn fallback_open_forwards_vm_id() {
    // Regression guard: the pre-refactor launch_ui added `--args` only
    // when vm_id was Some, and launch_ui_action always added `--args`.
    // Check both paths go through one helper with consistent behavior.
    let (program, args) = build_launch_invocation(None, Some("vm-9"), None);
    assert_eq!(program, os("open"));
    assert_eq!(
        args,
        vec![
            os("-a"),
            os("Capsem"),
            os("--args"),
            os("--connect"),
            os("vm-9")
        ]
    );
}

#[test]
fn fallback_open_forwards_vm_id_and_action() {
    let (program, args) = build_launch_invocation(None, Some("vm-9"), Some("fork"));
    assert_eq!(program, os("open"));
    assert_eq!(
        args,
        vec![
            os("-a"),
            os("Capsem"),
            os("--args"),
            os("--connect"),
            os("vm-9"),
            os("--action"),
            os("fork"),
        ]
    );
}

#[test]
fn start_service_invocation_uses_resolved_capsem_binary() {
    let binary = PathBuf::from("/usr/local/bin/capsem");
    let (program, args) = build_start_service_invocation(Some(&binary));
    assert_eq!(program, binary.as_os_str());
    assert_eq!(args, vec![os("start")]);
}

#[test]
fn start_service_invocation_falls_back_to_path_lookup() {
    let (program, args) = build_start_service_invocation(None);
    assert_eq!(program, os("capsem"));
    assert_eq!(args, vec![os("start")]);
}
