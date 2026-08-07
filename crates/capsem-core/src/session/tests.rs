use super::boot_failure_summary;

#[test]
fn boot_failure_summary_returns_the_last_line_that_says_something() {
    let tail = "INFO capsem_process: booting\n\
                INFO capsem_process: resolving assets\n\
                ERROR capsem_process: failed to build VmConfig: rootfs hash mismatch\n";

    assert_eq!(
        boot_failure_summary(tail),
        "ERROR capsem_process: failed to build VmConfig: rootfs hash mismatch"
    );
}

#[test]
fn boot_failure_summary_skips_the_blank_tail_a_crash_leaves_behind() {
    let tail = "ERROR capsem_process: vz launch rejected\n\r\n   \n\n";

    assert_eq!(
        boot_failure_summary(tail),
        "ERROR capsem_process: vz launch rejected"
    );
}

#[test]
fn boot_failure_summary_names_an_empty_log_rather_than_returning_nothing() {
    // A crash that produced no output must still log something an operator
    // can act on -- an empty string in a log line reads as a missing field.
    assert_eq!(boot_failure_summary(""), "(log empty)");
    assert_eq!(boot_failure_summary("  \n\n \r\n"), "(log empty)");
}
