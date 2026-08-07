//! The enumerator has to find real processes, and the cheapest real process
//! to assert about is the one running the test.

use super::running_processes;

#[test]
fn running_processes_finds_this_process_and_its_arguments() {
    let table = running_processes().expect("the process table must be readable");
    let mine = std::process::id();

    let line = table
        .lines()
        .find(|line| line.split_whitespace().next() == Some(&mine.to_string()))
        .unwrap_or_else(|| {
            panic!("the process table did not contain this test process (pid {mine})")
        });

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
