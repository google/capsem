// capsem-sysutil: Multi-call guest system binary for VM lifecycle commands.
//
// Dispatches on argv[0] (busybox pattern). Symlinked at boot by capsem-init:
//   /sbin/shutdown  -> /run/capsem-sysutil
//   /sbin/halt      -> /run/capsem-sysutil
//   /sbin/poweroff  -> /run/capsem-sysutil
//   /sbin/reboot    -> /run/capsem-sysutil
//   /usr/local/bin/suspend -> /run/capsem-sysutil
//
// Opens its own vsock:5004 connection directly (independent of capsem-pty-agent).
// This means shutdown works even if the agent is hung.

#[path = "../vsock_io.rs"]
mod vsock_io;

use std::io::{self, Write};
use std::process;
use std::thread;
use std::time::Duration;

use capsem_proto::{GuestToHost, SHUTDOWN_GRACE_SECS, VSOCK_PORT_LIFECYCLE, encode_guest_msg};
use vsock_io::{VSOCK_HOST_CID, write_all_fd};

fn countdown(label: &str) {
    let countdown_secs = SHUTDOWN_GRACE_SECS as u32 + 1;
    for i in (1..=countdown_secs).rev() {
        eprint!("\r[capsem] {label} in {i}...");
        let _ = io::stderr().flush();
        thread::sleep(Duration::from_secs(1));
    }
    eprintln!("\r[capsem] {label}...        ");
}

fn send_lifecycle_msg(msg: &GuestToHost) -> io::Result<()> {
    let fd = vsock_io::vsock_connect(VSOCK_HOST_CID, VSOCK_PORT_LIFECYCLE)?;
    let frame = match encode_guest_msg(msg) {
        Ok(f) => f,
        Err(e) => { unsafe { nix::libc::close(fd); } return Err(io::Error::other(e)); }
    };
    let res = write_all_fd(fd, &frame);
    unsafe { nix::libc::close(fd); }
    res
}

/// Extract the command name from argv[0], stripping path prefixes.
fn command_name(argv0: &str) -> &str {
    argv0.rsplit('/').next().unwrap_or(argv0)
}

/// Check if this is a reboot request (shutdown -r or direct reboot invocation).
fn is_reboot_request(cmd: &str, args: &[String]) -> bool {
    if cmd == "reboot" {
        return true;
    }
    // Only "shutdown -r" means reboot. halt/poweroff don't support -r.
    cmd == "shutdown" && args.iter().any(|a| a == "-r")
}

fn print_help(cmd: &str) {
    println!("Usage: {cmd} [OPTIONS]");
    println!("Capsem sandbox lifecycle command.");
    println!();
    match cmd {
        "shutdown" | "halt" | "poweroff" => {
            println!("Stops the sandbox cleanly through the host service.");
            println!("Accepted flags: -h, -P (default behavior), -r (error: reboot not supported)");
        }
        "suspend" => {
            println!("Suspends the sandbox (persistent VMs only).");
            println!("Use 'capsem resume <name>' on the host to restore.");
        }
        "reboot" => {
            println!("Reboot is not supported in capsem sandbox.");
        }
        _ => {
            println!("Commands: shutdown, halt, poweroff, reboot, suspend");
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = command_name(&args[0]);

    // Handle --help for any command
    if args.iter().any(|a| a == "--help" || (a == "-h" && cmd != "shutdown")) {
        print_help(cmd);
        process::exit(0);
    }

    match cmd {
        "shutdown" | "halt" | "poweroff" => {
            if is_reboot_request(cmd, &args[1..]) {
                eprintln!("[capsem] reboot is not supported in capsem sandbox");
                process::exit(1);
            }
            countdown("Shutting down");
            if let Err(e) = send_lifecycle_msg(&GuestToHost::ShutdownRequest) {
                eprintln!("[capsem] failed to send shutdown request: {e}");
                process::exit(1);
            }
        }
        "reboot" => {
            eprintln!("[capsem] reboot is not supported in capsem sandbox");
            process::exit(1);
        }
        "suspend" => {
            countdown("Suspending");
            if let Err(e) = send_lifecycle_msg(&GuestToHost::SuspendRequest) {
                eprintln!("[capsem] failed to send suspend request: {e}");
                process::exit(1);
            }
        }
        _ => {
            // Direct invocation as capsem-sysutil
            if args.len() > 1 {
                match args[1].as_str() {
                    "shutdown" | "halt" | "poweroff" => {
                        countdown("Shutting down");
                        if let Err(e) = send_lifecycle_msg(&GuestToHost::ShutdownRequest) {
                            eprintln!("[capsem] failed to send shutdown request: {e}");
                            process::exit(1);
                        }
                    }
                    "suspend" => {
                        countdown("Suspending");
                        if let Err(e) = send_lifecycle_msg(&GuestToHost::SuspendRequest) {
                            eprintln!("[capsem] failed to send suspend request: {e}");
                            process::exit(1);
                        }
                    }
                    "reboot" => {
                        eprintln!("[capsem] reboot is not supported in capsem sandbox");
                        process::exit(1);
                    }
                    "--help" => {
                        print_help("capsem-sysutil");
                        process::exit(0);
                    }
                    other => {
                        eprintln!("[capsem] unknown command: {other}");
                        eprintln!("Available: shutdown, halt, poweroff, reboot, suspend");
                        process::exit(1);
                    }
                }
            } else {
                print_help("capsem-sysutil");
                process::exit(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
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
}
