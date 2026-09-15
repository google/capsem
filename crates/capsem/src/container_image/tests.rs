use super::*;
use crate::client::tests::fake_service::FakeService;
use serde_json::json;

fn image_args(publish: &[&str]) -> ImageArgs {
    ImageArgs {
        image: vec!["docker://redis:7".into(), "redis-server".into(), "--save".into()],
        publish: publish.iter().map(|mapping| mapping.parse().unwrap()).collect(),
        ..ImageArgs::default()
    }
}

fn status(state: &str, digest: Option<&str>) -> serde_json::Value {
    let mut status = json!({"state": state, "image": "docker://redis:7"});
    if let Some(digest) = digest {
        status["digest"] = json!(digest);
    }
    status
}

#[tokio::test]
async fn provision_leaves_resources_to_the_profile_when_unset() {
    let service = FakeService::start();
    service.route(
        "POST",
        "/vms/create",
        200,
        json!({"id": "vm-1", "name": "vm-1", "profile_id": "code", "status": "Running", "available_actions": []}),
    );
    let request = ProvisionRequest {
        name: None,
        profile_id: "code".into(),
        ram_mb: None,
        cpus: None,
        persistent: false,
        env: None,
        from: None,
        networks: vec![],
        container: None,
    };
    assert_eq!(provision(&service.client, &request).await.unwrap().id, "vm-1");
    let body = service.find("POST", "/vms/create")[0].json();
    assert!(
        body.get("ram_mb").is_none() && body.get("cpus").is_none() && body.get("container").is_none(),
        "{body}"
    );
}

#[tokio::test]
async fn destroy_reports_what_the_service_refused() {
    let service = FakeService::start();
    service
        .once("DELETE", "/vms/vm-1/delete", 200, json!({"success": true}))
        .route("DELETE", "/vms/vm-1/delete", 404, json!({"error": "no such vm"}));
    destroy(&service.client, "vm-1").await.unwrap();
    let refused = destroy(&service.client, "vm-1").await.unwrap_err();
    assert!(format!("{refused:#}").contains("no such vm"), "{refused:#}");
}

#[tokio::test]
async fn the_spec_carries_the_command_environment_and_attach() {
    let args = image_args(&[]);
    let env = ["B=2".to_string(), "A=1".to_string()];
    let workload = Workload::of(&args, &env).unwrap().unwrap();
    let spec = workload.spec(true).await.unwrap();
    assert_eq!(spec.image, "docker://redis:7");
    assert_eq!(spec.args, ["redis-server", "--save"]);
    assert_eq!(
        spec.env.iter().collect::<Vec<_>>(),
        [
            (&"A".to_string(), &"1".to_string()),
            (&"B".to_string(), &"2".to_string())
        ]
    );
    assert!(spec.attach);
    assert_eq!(spec.registry, None, "an anonymous pull sends no registry access");
}

#[tokio::test]
async fn the_spec_refuses_what_it_cannot_name_before_any_vm_exists() {
    let args = ImageArgs {
        image: vec!["not a reference!".into()],
        ..ImageArgs::default()
    };
    let workload = Workload::of(&args, &[]).unwrap().unwrap();
    let error = workload.spec(false).await.err().unwrap();
    assert!(format!("{error:#}").contains("--image expects"), "{error:#}");
}

#[test]
fn registry_access_needs_a_password_and_carries_the_ca() {
    let _lock = crate::lock_test_env();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let certificate = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(certificate.path(), "-----BEGIN CERTIFICATE-----\n").unwrap();
    let args = ImageArgs {
        image: vec!["docker://redis:7".into()],
        registry_user: Some("me".into()),
        registry_ca: Some(certificate.path().to_path_buf()),
        ..ImageArgs::default()
    };
    let workload = Workload::of(&args, &[]).unwrap().unwrap();

    std::env::remove_var("CAPSEM_REGISTRY_PASSWORD");
    let error = runtime.block_on(workload.spec(false)).err().unwrap();
    assert!(format!("{error:#}").contains("CAPSEM_REGISTRY_PASSWORD"), "{error:#}");

    std::env::set_var("CAPSEM_REGISTRY_PASSWORD", "s3cret");
    let spec = runtime.block_on(workload.spec(false));
    std::env::remove_var("CAPSEM_REGISTRY_PASSWORD");
    let registry = spec.unwrap().registry.unwrap();
    assert_eq!(registry.username.as_deref(), Some("me"));
    assert_eq!(registry.password.as_deref(), Some("s3cret"));
    assert_eq!(registry.ca_pem.as_deref(), Some("-----BEGIN CERTIFICATE-----\n"));
}

#[tokio::test]
async fn follow_waits_for_the_state_it_needs() {
    let service = FakeService::start();
    service
        .once("GET", "/vms/vm-1/container", 200, status("pulling", None))
        .once("GET", "/vms/vm-1/container", 200, status("staging", Some("sha256:ab")))
        .route("GET", "/vms/vm-1/container", 200, status("staged", Some("sha256:ab")));
    let staged = follow(&service.client, "vm-1", "docker://redis:7", |state| {
        state == ContainerState::Staged
    })
    .await
    .unwrap();
    assert_eq!(staged.digest.as_deref(), Some("sha256:ab"));
    assert_eq!(service.find("GET", "/vms/vm-1/container").len(), 3);
}

#[tokio::test]
async fn a_failed_setup_is_an_error_with_the_services_reason() {
    let service = FakeService::start();
    let mut failed = status("failed", None);
    failed["error"] = json!("pull docker://redis:7: certificate unknown");
    service.route("GET", "/vms/vm-1/container", 200, failed);
    let error = follow(&service.client, "vm-1", "docker://redis:7", |_| false)
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("certificate unknown"), "{error:#}");
}

#[tokio::test]
async fn expose_asks_the_service_for_each_mapping_and_stops_at_a_refusal() {
    let service = FakeService::start();
    service
        .once(
            "POST",
            "/vms/vm-1/exposures",
            200,
            json!({"id": "4100", "host_port": 4100, "guest_port": 6379, "target": "container"}),
        )
        .route(
            "POST",
            "/vms/vm-1/exposures",
            409,
            json!({"error": "host port 9099 is in use"}),
        );
    let args = image_args(&["0:6379", "9099:9099", "0:80"]);
    let error = expose(&service.client, "vm-1", &args.publish).await.unwrap_err();
    assert!(format!("{error:#}").contains("in use"), "{error:#}");
    let requests = service.find("POST", "/vms/vm-1/exposures");
    assert_eq!(requests.len(), 2, "a refused mapping stops the rest");
    assert_eq!(
        requests[1].json(),
        json!({"guest_port": 9099, "host_port": 9099, "target": "container"})
    );
}
