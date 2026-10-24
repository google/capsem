use super::*;
use capsem_foundation::unix::process::{self, ProcessId, ProcessState};

fn plan(script: String) -> api::UpdateCommandPlan {
    api::UpdateCommandPlan {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script],
    }
}

fn limits() -> Limits {
    Limits {
        timeout: Duration::from_secs(1),
        output: 32,
    }
}

#[tokio::test]
async fn bounded_apply_capture_drains_both_pipes_through_actual_child_settlement() {
    let dir = tempfile::tempdir().unwrap();
    let done = dir.path().join("done");
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let command = plan(format!(
        "printf '%0100d' 0; printf '%0100d' 0 >&2; printf done > '{}'",
        done.display()
    ));
    let error = run(&command, UpdateCommandKind::Apply, cancelled, limits())
        .await
        .unwrap();
    assert!(error.status.success());
    let error = error.output.unwrap_err();
    assert!(error.to_string().contains("byte limit"));
    assert_eq!(
        std::fs::read(&done).unwrap(),
        b"done",
        "output overflow or caller cancellation interrupted the transaction"
    );
}

#[tokio::test]
async fn check_deadline_kills_and_reaps_the_exact_child() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    let command = plan(format!("printf '%s' $$ > '{}'; exec sleep 30", pid_file.display()));
    let error = run(&command, UpdateCommandKind::Check, CancellationToken::new(), limits())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    let pid = ProcessId::try_from(std::fs::read_to_string(pid_file).unwrap().parse::<u32>().unwrap()).unwrap();
    assert_eq!(process::probe(pid).unwrap(), ProcessState::Gone);
}

#[tokio::test]
async fn cancelled_check_never_starts_a_child() {
    let dir = tempfile::tempdir().unwrap();
    let effect = dir.path().join("effect");
    let token = CancellationToken::new();
    token.cancel();
    let error = run(
        &plan(format!("printf bad > '{}'", effect.display())),
        UpdateCommandKind::Check,
        token,
        limits(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert!(!effect.exists());
}

#[tokio::test]
async fn bounded_capture_preserves_complete_success_and_failed_exit() {
    for kind in [UpdateCommandKind::Check, UpdateCommandKind::Apply] {
        for code in [0, 7] {
            let output = run(
                &plan(format!("printf complete; printf detail >&2; exit {code}")),
                kind,
                CancellationToken::new(),
                limits(),
            )
            .await
            .unwrap()
            .output
            .unwrap();
            assert_eq!(output.status.code(), Some(code));
            assert_eq!(output.stdout, b"complete");
            assert_eq!(output.stderr, b"detail");
        }
    }
}

#[tokio::test]
async fn capture_propagates_reader_and_spawn_errors() {
    struct Broken;
    impl AsyncRead for Broken {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            _: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            std::task::Poll::Ready(Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "synthetic pipe failure",
            )))
        }
    }
    for drain in [false, true] {
        assert_eq!(
            read_pipe(Broken, 8, drain).await.unwrap_err().kind(),
            io::ErrorKind::ConnectionReset
        );
    }
    let (mut writer, reader) = tokio::io::duplex(32);
    use tokio::io::AsyncWriteExt;
    writer.write_all(b"01234567890123456789012345678901").await.unwrap();
    drop(writer);
    let (bytes, exceeded) = read_pipe(reader, 8, true).await.unwrap();
    assert_eq!(bytes, b"01234567");
    assert!(exceeded);
    let command = api::UpdateCommandPlan {
        program: "/capsem-missing-update-test-child".into(),
        args: vec![],
    };
    let error = run(&command, UpdateCommandKind::Check, CancellationToken::new(), limits())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}
