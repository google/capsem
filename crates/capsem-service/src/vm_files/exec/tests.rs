use super::*;
use std::future::Future;

#[tokio::test]
async fn replacement_during_exec_preparation_never_receives_the_command() {
    let state = crate::tests::make_test_state();
    let tmp = tempfile::tempdir().unwrap();
    crate::tests::insert_fake_instance_with_session_dir(&state, "owner", std::process::id(), tmp.path().to_owned());
    let path = state.instances.lock().unwrap()["owner"].uds_path.clone();
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let mut execution = Box::pin(handle_exec(
        crate::tests::operator_actor(),
        State(Arc::clone(&state)),
        Path("owner".into()),
        Json(ExecRequest {
            command: "must-not-run".into(),
            timeout_secs: Some(1),
            target: None,
        }),
    ));
    // Poll once: admission has run, while container/readiness preparation
    // must yield. Replace the generation before that continuation resumes.
    std::future::poll_fn(|cx| {
        assert!(execution.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    state.instances.lock().unwrap().get_mut("owner").unwrap().generation = capsem_crypto::uuid_v4();
    std::fs::write(path.with_extension("ready"), "ready").unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::select! {
            result = execution => {
                let error = result.expect_err("exec must reject the replacement generation");
                assert!(error.body.error.contains("VM owner changed"), "{}", error.body.error);
            }
            accepted = listener.accept() => {
                accepted.unwrap();
                panic!("exec contacted the replacement owner");
            }
        }
    })
    .await
    .expect("exec must reject replacement promptly");
}
