use super::*;

#[test]
fn command_name_strips_path() {
    assert_eq!(command_name("/sbin/shutdown"), "shutdown");
    assert_eq!(command_name("/usr/local/bin/suspend"), "suspend");
    assert_eq!(command_name("halt"), "halt");
    assert_eq!(command_name("/run/capsem-sysutil"), "capsem-sysutil");
}

#[test]
fn reboot_detection() {
    assert!(is_reboot_request("reboot", &[]));
    assert!(is_reboot_request("shutdown", &["-r".into()]));
    assert!(is_reboot_request("shutdown", &["-r".into(), "now".into()]));
    assert!(!is_reboot_request("shutdown", &[]));
    assert!(!is_reboot_request("shutdown", &["-h".into(), "now".into()]));
    assert!(!is_reboot_request("halt", &[]));
    assert!(!is_reboot_request("poweroff", &[]));
}

#[test]
fn reboot_flag_not_in_halt_or_poweroff() {
    // -r should only trigger reboot when cmd is "shutdown"
    assert!(!is_reboot_request("halt", &["-r".into()]));
    assert!(!is_reboot_request("poweroff", &["-r".into()]));
}

#[test]
fn command_name_handles_empty_string() {
    assert_eq!(command_name(""), "");
}

#[test]
fn command_name_multiple_slashes() {
    assert_eq!(command_name("///shutdown"), "shutdown");
    assert_eq!(command_name("/a/b/c/d/halt"), "halt");
}
