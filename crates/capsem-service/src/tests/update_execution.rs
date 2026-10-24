use super::*;
use capsem_foundation::unix::process::{self, ProcessId, ProcessState};
use std::time::Duration;

fn plan(script: String) -> api::UpdateCommandPlan {
    api::UpdateCommandPlan {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script],
    }
}

async fn exists(path: &StdPath) -> bool {
    capsem_foundation::poll::poll_until(
        capsem_foundation::poll::PollOpts::new("update child effect", Duration::from_secs(3)),
        || async { path.exists().then_some(()) },
    )
    .await
    .is_ok()
}

#[tokio::test]
async fn update_check_refuses_output_exceeding_configured_capture() {
    let state = make_test_state();
    let result = execute_update_command(&state, plan("printf '%070000d' 0".into())).await;
    assert!(
        result.is_err(),
        "oversized output must not be returned as complete success"
    );
}

#[tokio::test]
async fn update_check_caller_drop_reaps_before_releasing_serialization() {
    let state = make_test_state();
    let dir = tempfile::tempdir().unwrap();
    let started = dir.path().join("started");
    let script = format!("printf '%s' $$ > '{}'; exec sleep 30", started.display());
    let owned_state = Arc::clone(&state);
    let caller = tokio::spawn(async move { execute_update_command(&owned_state, plan(script)).await });
    assert!(exists(&started).await);
    let pid = ProcessId::try_from(std::fs::read_to_string(&started).unwrap().parse::<u32>().unwrap()).unwrap();
    caller.abort();
    let _ = caller.await;
    let settled = capsem_foundation::poll::poll_until(
        capsem_foundation::poll::PollOpts::new("cancelled check settled", Duration::from_secs(3)),
        || async { state.update_lock.try_lock().ok().map(|_| process::probe(pid).unwrap()) },
    )
    .await;
    if process::probe(pid).unwrap() != ProcessState::Gone {
        process::send_signal(pid, process::Signal::Kill).unwrap();
    }
    assert_eq!(
        settled.unwrap(),
        ProcessState::Gone,
        "check lock released before child reaping"
    );
}

#[tokio::test]
async fn update_apply_caller_drop_keeps_transaction_pipes_and_serialization() {
    let state = make_test_state();
    let dir = tempfile::tempdir().unwrap();
    let started = dir.path().join("started");
    let release = dir.path().join("release");
    let done = dir.path().join("done");
    let script = format!(
        "printf '%s' $$ > '{}'; while [ ! -f '{}' ]; do sleep 0.01; done; printf survived; printf done > '{}'",
        started.display(),
        release.display(),
        done.display(),
    );
    let owned_state = Arc::clone(&state);
    let caller = tokio::spawn(async move { execute_update_apply(&owned_state, plan(script)).await });
    let admitted = exists(&started).await;
    caller.abort();
    let _ = caller.await;
    let serialized = state.update_lock.try_lock().is_err();
    std::fs::write(&release, b"release").unwrap();
    let settled = exists(&done).await;
    if admitted {
        let pid = std::fs::read_to_string(&started).unwrap().parse::<u32>().unwrap();
        let pid = ProcessId::try_from(pid).unwrap();
        let gone = capsem_foundation::poll::poll_until(
            capsem_foundation::poll::PollOpts::new("update child reaped", Duration::from_secs(3)),
            || async { (process::probe(pid).unwrap() == ProcessState::Gone).then_some(()) },
        )
        .await;
        if gone.is_err() {
            process::send_signal(pid, process::Signal::Kill).unwrap();
        }
        assert!(gone.is_ok(), "owned updater child did not settle");
    }
    assert!(admitted, "updater was never admitted");
    assert!(serialized, "caller cancellation released an active transaction lock");
    assert!(settled, "caller cancellation closed the real updater's output pipe");
}
