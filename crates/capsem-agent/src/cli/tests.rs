use super::*;

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn only_an_argument_free_start_runs_the_agent() {
    assert_eq!(cli_action(&[]), CliAction::Run);
}

#[test]
fn help_and_version_answer_without_running() {
    for flag in ["--help", "-h"] {
        let CliAction::Exit { code, message } = cli_action(&args(&[flag])) else {
            panic!("{flag} must not run the agent");
        };
        assert_eq!(code, 0);
        assert!(message.contains("Usage: capsem-pty-agent"), "{message}");
    }
    for flag in ["--version", "-V"] {
        assert_eq!(
            cli_action(&args(&[flag])),
            CliAction::Exit {
                code: 0,
                message: format!("capsem-pty-agent {}", env!("CARGO_PKG_VERSION"))
            }
        );
    }
}

/// Anything unexpected is refused, never treated as a start: an unknown flag,
/// extra arguments, or a flag after a valid one.
#[test]
fn every_other_command_line_is_refused() {
    for line in [
        &["--bogus"][..],
        &["--help", "--version"],
        &["run"],
        &[""],
        &["--help", "x"],
    ] {
        let action = cli_action(&args(line));
        assert!(
            matches!(action, CliAction::Exit { code: 2, .. }),
            "{line:?} -> {action:?}"
        );
    }
}
