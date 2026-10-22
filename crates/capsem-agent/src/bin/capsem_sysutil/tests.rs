use super::*;

#[test]
fn browser_helper_sends_the_shared_bounded_messagepack_request() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("browser.sock");
    let receiver = std::os::unix::net::UnixDatagram::bind(&path).unwrap();
    receiver.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    open_browser(&path, &["https://example.org/login?state=opaque".into()]).unwrap();
    let mut buffer = [0; 9000];
    let size = receiver.recv(&mut buffer).unwrap();
    assert_eq!(u32::from_be_bytes(buffer[..4].try_into().unwrap()) as usize, size - 4);
    assert!(
        matches!(capsem_proto::decode_guest_msg(&buffer[4..size]).unwrap(), GuestToHost::OpenUrl { url } if url.0 == "https://example.org/login?state=opaque")
    );
    assert!(open_browser(&path, &["file:///etc/passwd".into()]).is_err());
    assert!(open_browser(&path, &[]).is_err());
}

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

#[test]
fn help_covers_each_command_family() {
    for command in ["shutdown", "halt", "poweroff", "suspend", "reboot", "capsem-sysutil"] {
        print_help(command);
    }
}

#[test]
fn lifecycle_send_fails_cleanly_without_a_host_vsock() {
    let error = send_lifecycle_msg(&GuestToHost::ShutdownRequest).unwrap_err();
    assert_ne!(error.kind(), io::ErrorKind::InvalidData);
}
