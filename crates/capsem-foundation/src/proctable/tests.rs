//! The shared process enumerator has to find real processes; the cheapest one
//! to assert about is the one running the test.

use super::{processes, resident_bytes, running_processes};

#[test]
fn running_processes_finds_this_process_and_its_arguments() {
    let table = running_processes().expect("the process table must be readable");
    let mine = std::process::id();

    let line = table
        .lines()
        .find(|line| line.split_whitespace().next() == Some(&mine.to_string()))
        .unwrap_or_else(|| panic!("the process table did not contain this test process (pid {mine})"));

    // argv[0], not just the pid: the orphan match is on `--session-dir`, so an
    // enumerator that reports pids with empty command lines would satisfy a
    // weaker assertion here and still reap nothing.
    let arguments = line.split_once(' ').expect("a pid with no argv is useless").1;
    assert!(
        !arguments.trim().is_empty(),
        "this process was listed with no arguments: {line:?}"
    );
}

#[test]
fn process_records_carry_real_parent_identity() {
    let mine = std::process::id();
    let process = processes()
        .expect("the process table must be readable")
        .into_iter()
        .find(|process| process.pid == mine)
        .unwrap_or_else(|| panic!("the process table did not contain pid {mine}"));

    assert!(process.parent_pid > 0);
    assert!(!process.arguments.is_empty());
}

#[test]
fn running_processes_does_not_leak_the_environment() {
    // `KERN_PROCARGS2` returns argv *and* environ. Stopping at argc is what
    // keeps secrets out of a string this service logs and matches against.
    let sentinel = "CAPSEM_PROCTABLE_SENTINEL";
    // SAFETY: single-threaded at this point in the test binary, and the value
    // is only ever read back through the enumerator below.
    unsafe { std::env::set_var(sentinel, "must-not-appear") };

    let table = running_processes().expect("the process table must be readable");

    assert!(
        !table.contains("must-not-appear"),
        "the enumerator included the environment, which carries secrets"
    );
}

/// A resident size read with a syscall, so a sandboxed caller gets it: the
/// router's memory proof shelled out to setuid `ps` and failed under the gate.
#[test]
fn resident_bytes_reads_a_live_process_and_refuses_a_gone_one() {
    let ballast = vec![7u8; 32 * 1024 * 1024];
    let resident = resident_bytes(std::process::id()).expect("this process is readable");
    assert!(
        resident >= ballast.len() as u64,
        "{resident} bytes resident with 32 MiB touched"
    );
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let gone = child.id();
    child.wait().unwrap();
    assert!(resident_bytes(gone).is_err(), "a reaped pid has no resident size");
}
