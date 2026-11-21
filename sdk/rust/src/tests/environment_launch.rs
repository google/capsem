use super::*;

#[tokio::test]
async fn lifecycle_mutations_carry_the_original_selection_from_the_requested_intent() {
    use axum::{body::Body, http::Response};

    let selected = json!({
        "id": "vm-1",
        "generation": "00000000-0000-4000-8000-000000000003",
        "running": false,
        "environment": {
            "environment_id": "00000000-0000-4000-8000-000000000001",
            "revision_id": "00000000-0000-4000-8000-000000000002",
        },
    });
    let metadata = json!({"selection": selected, "startup_timeout_seconds": 600});
    let mut server = Server::respond(move |parts| {
        let result = match parts.uri.path() {
            "/vms/vm-1/readiness" => metadata.clone(),
            "/vms/vm-1/start" => reply("startVm"),
            "/vms/vm-1/resume" => reply("resumeVm"),
            "/vms/vm-1/fork" => reply("forkVm"),
            path => panic!("unexpected request: {path}"),
        };
        Response::new(Body::from(result.to_string()))
    })
    .await;
    let vm = VM::new(&server.url, "private-token", VmSelector::Id("vm-1".into())).unwrap();
    vm.start().await.unwrap();
    vm.resume().await.unwrap();
    vm.fork("copy", None, None).await.unwrap();
    for (operation, intent) in [("start", "start"), ("resume", "start"), ("fork", "fork")] {
        let (metadata, body) = server.received.recv().await.unwrap();
        assert_eq!(metadata.method, "GET");
        assert_eq!(metadata.headers["authorization"], "Bearer private-token");
        assert_eq!(metadata.uri.to_string(), format!("/vms/vm-1/readiness?intent={intent}"));
        assert!(body.is_empty());
        let request = request(&mut server, &format!("/vms/vm-1/{operation}")).await;
        assert_eq!(request["expected"], selected);
    }
    assert!(server.received.try_recv().is_err());
}

#[tokio::test]
async fn invalid_lifecycle_selection_does_not_dispatch_a_mutation() {
    use axum::{body::Body, http::Response};

    let mut server = Server::respond(|_| {
        Response::new(Body::from(
            json!({
                "selection": {"id":"other-vm","generation":null,"running":false,"environment":null},
                "startup_timeout_seconds":600,
            })
            .to_string(),
        ))
    })
    .await;
    let vm = VM::new(&server.url, "private-token", VmSelector::Id("vm-1".into())).unwrap();
    assert!(matches!(vm.resume().await, Err(Error::InvalidResponse(_))));
    request(&mut server, "/vms/vm-1/readiness").await;
    assert!(server.received.try_recv().is_err());
}

#[tokio::test]
async fn create_from_binds_the_source_and_preserves_the_destination_override() {
    use axum::{body::Body, http::Response};

    let inherited = json!({
        "environment_id": "00000000-0000-4000-8000-000000000001",
        "revision_id": "00000000-0000-4000-8000-000000000002",
    });
    let selected = json!({
        "id": "vm-1", "generation": "00000000-0000-4000-8000-000000000003",
        "running": false, "environment": inherited,
    });
    let override_reference = json!({
        "environment_id": "00000000-0000-4000-8000-000000000001",
        "revision_id": "00000000-0000-4000-8000-000000000004",
    });
    let readiness_path =
        "/environments/00000000-0000-4000-8000-000000000001/revisions/00000000-0000-4000-8000-000000000004/readiness";
    let source = json!({"selection": selected, "startup_timeout_seconds": 600});
    let destination = json!({"environment": override_reference, "startup_timeout_seconds": 20});
    let mut server = Server::respond(move |parts| {
        let response = match parts.uri.path() {
            "/vms/source-name/readiness" => source.clone(),
            path if path == readiness_path => destination.clone(),
            "/vms/create" => reply("createVm"),
            path => panic!("unexpected request: {path}"),
        };
        Response::new(Body::from(response.to_string()))
    })
    .await;
    let hv = Hypervisor::new(&server.url, "private-token").unwrap();
    for override_destination in [false, true] {
        let environment = override_destination.then(|| serde_json::from_value(override_reference.clone()).unwrap());
        hv.create(CreateOptions {
            from: Some("source-name".into()),
            environment,
            ..Default::default()
        })
        .await
        .unwrap();
        let (parts, payload) = server.received.recv().await.unwrap();
        assert_eq!(parts.uri, "/vms/source-name/readiness?intent=clone");
        assert!(payload.is_empty());
        if override_destination {
            request(&mut server, readiness_path).await;
        }
        let body = request(&mut server, "/vms/create").await;
        assert_eq!(body["from"], "source-name");
        assert_eq!(body["expected_source"], selected);
        assert_eq!(
            &body["environment"],
            if override_destination {
                &override_reference
            } else {
                &serde_json::Value::Null
            }
        );
    }
    assert!(server.received.try_recv().is_err());
}

#[tokio::test]
async fn environment_launch_preserves_revision_and_refuses_inline_overrides_before_http() {
    let mut server = gateway().await;
    let hv = Hypervisor::new(&server.url, "private-token").unwrap();
    let environment = capsem_api::environments::EnvironmentRevisionReference {
        environment_id: "00000000-0000-4000-8000-000000000001".into(),
        revision_id: "00000000-0000-4000-8000-000000000002".into(),
    };
    let vm = hv
        .create(CreateOptions {
            environment: Some(environment.clone()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(vm.id(), Some("vm-1"));
    request(
        &mut server,
        "/environments/00000000-0000-4000-8000-000000000001/revisions/00000000-0000-4000-8000-000000000002/readiness",
    )
    .await;
    let body = request(&mut server, "/vms/create").await;
    assert!(
        body.get("attach").is_none(),
        "default detached requests preserve their wire shape"
    );
    assert_eq!(
        body["environment"],
        json!({"environment_id": environment.environment_id, "revision_id": environment.revision_id})
    );
    for options in [
        CreateOptions {
            environment: Some(environment.clone()),
            env: Some(Default::default()),
            ..Default::default()
        },
        CreateOptions {
            environment: Some(environment),
            image: Some("busybox".into()),
            ..Default::default()
        },
    ] {
        assert!(matches!(hv.create(options).await, Err(Error::InvalidInput(_))));
    }
    assert!(server.received.try_recv().is_err());
}

#[tokio::test]
async fn attached_saved_environment_selects_only_transport_and_requires_a_reference() {
    let mut server = gateway().await;
    let hv = Hypervisor::new(&server.url, "private-token").unwrap();
    let environment = capsem_api::environments::EnvironmentRevisionReference {
        environment_id: "00000000-0000-4000-8000-000000000001".into(),
        revision_id: "00000000-0000-4000-8000-000000000002".into(),
    };
    hv.create(CreateOptions {
        environment: Some(environment.clone()),
        attach: true,
        ..Default::default()
    })
    .await
    .unwrap();
    let body = request(&mut server, "/vms/create").await;
    assert_eq!(body["attach"], true);
    assert_eq!(body["environment"]["revision_id"], environment.revision_id);
    assert!(body.get("container").is_none() && body.get("env").is_none());
    assert!(matches!(
        hv.create(CreateOptions {
            attach: true,
            ..Default::default()
        })
        .await,
        Err(Error::InvalidInput(_))
    ));
    assert!(server.received.try_recv().is_err());
}

#[tokio::test]
async fn saved_creation_reads_exact_timing_before_mutation_and_refuses_changed_or_unavailable_metadata() {
    use axum::{body::Body, http::Response};
    for outcome in ["verified", "changed", "unavailable", "overflow"] {
        let mut server = Server::respond(move |parts| {
            if parts.uri.path().ends_with("/readiness") {
                if outcome == "unavailable" {
                    return Response::builder().status(403).body(Body::from("inspection refused")).unwrap();
                }
                return Response::new(Body::from(json!({
                    "environment": {"environment_id": "00000000-0000-4000-8000-000000000001",
                        "revision_id": if outcome == "changed" { "00000000-0000-4000-8000-000000000003" } else { "00000000-0000-4000-8000-000000000002" }},
                    "startup_timeout_seconds": if outcome == "overflow" { u64::from(u32::MAX) + 1 } else { 600 }
                }).to_string()));
            }
            assert_eq!(parts.uri.path(), "/vms/create");
            Response::new(Body::from(reply("createVm").to_string()))
        }).await;
        let hv = Hypervisor::new(&server.url, "private-token").unwrap();
        let result = hv
            .create(CreateOptions {
                environment: Some(capsem_api::environments::EnvironmentRevisionReference {
                    environment_id: "00000000-0000-4000-8000-000000000001".into(),
                    revision_id: "00000000-0000-4000-8000-000000000002".into(),
                }),
                ..Default::default()
            })
            .await;
        assert_eq!(result.is_ok(), outcome == "verified");
        request(&mut server, "/environments/00000000-0000-4000-8000-000000000001/revisions/00000000-0000-4000-8000-000000000002/readiness").await;
        if outcome == "verified" {
            request(&mut server, "/vms/create").await;
        }
        assert!(
            server.received.try_recv().is_err(),
            "metadata refusal must never allocate a VM or replay creation"
        );
    }
}

#[tokio::test]
async fn saved_creation_options_preserve_full_approved_wait_and_larger_explicit_default() {
    use crate::transport::CallOptions;
    use axum::{body::Body, http::Response};
    for seconds in [0_u32, 600, u32::MAX] {
        let server = Server::respond(move |_| Response::new(Body::from(json!({
            "environment": {"environment_id": "00000000-0000-4000-8000-000000000001", "revision_id": "00000000-0000-4000-8000-000000000002"},
            "startup_timeout_seconds": seconds
        }).to_string()))).await;
        let body = models::ProvisionRequest {
            environment: Some(capsem_api::environments::EnvironmentRevisionReference {
                environment_id: "00000000-0000-4000-8000-000000000001".into(),
                revision_id: "00000000-0000-4000-8000-000000000002".into(),
            }),
            ..Default::default()
        };
        let planned = create_call_options(&server.client(), &body, CallOptions::default())
            .await
            .unwrap();
        assert_eq!(planned.timeout, Some(Duration::from_secs(230 + u64::from(seconds))));
        let larger = Duration::from_secs(10_000_000_000);
        let planned = create_call_options(&server.client(), &body, CallOptions { timeout: Some(larger) })
            .await
            .unwrap();
        assert_eq!(planned.timeout, Some(larger));
    }
}

#[tokio::test]
async fn cancelling_managed_timing_read_never_admits_creation_or_cleanup() {
    use axum::{
        body::{Body, Bytes},
        http::Response,
    };
    let mut server = Server::respond(|parts| {
        assert!(parts.uri.path().ends_with("/readiness"));
        Response::new(Body::from_stream(futures::stream::pending::<
            std::result::Result<Bytes, std::io::Error>,
        >()))
    })
    .await;
    let hv = Hypervisor::new(&server.url, "private-token").unwrap();
    let task = tokio::spawn(async move {
        hv.with_managed(
            CreateOptions {
                environment: Some(capsem_api::environments::EnvironmentRevisionReference {
                    environment_id: "00000000-0000-4000-8000-000000000001".into(),
                    revision_id: "00000000-0000-4000-8000-000000000002".into(),
                }),
                ..Default::default()
            },
            |_, _| async { Ok(()) },
        )
        .await
    });
    request(
        &mut server,
        "/environments/00000000-0000-4000-8000-000000000001/revisions/00000000-0000-4000-8000-000000000002/readiness",
    )
    .await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(server.received.try_recv().is_err());
}
