use super::*;

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
