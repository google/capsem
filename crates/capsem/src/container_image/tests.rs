use super::*;
use crate::client::tests::fake_service::FakeService;
use serde_json::json;
use sha2::{Digest, Sha256};

fn vm(uds_path: Option<&str>) -> ProvisionResponse {
    serde_json::from_value(json!({
        "id": "vm-1", "name": "vm-1", "profile_id": "code", "status": "Running",
        "available_actions": [], "uds_path": uds_path,
    }))
    .unwrap()
}

fn exec(exit_code: i32, stderr: &str) -> serde_json::Value {
    json!({"stdout": "", "stderr": stderr, "exit_code": exit_code})
}

fn image_args(publish: &[&str]) -> ImageArgs {
    ImageArgs {
        image: vec!["docker://redis:7".into(), "redis-server".into(), "--save".into()],
        publish: publish.iter().map(|mapping| mapping.parse().unwrap()).collect(),
        ..ImageArgs::default()
    }
}

/// An image layout on disk: one blob of `size` bytes.
fn layout(size: usize) -> (tempfile::TempDir, Vec<std::path::PathBuf>, Vec<u8>) {
    let root = tempfile::tempdir().unwrap();
    let relative = std::path::PathBuf::from("blobs/sha256/aa");
    std::fs::create_dir_all(root.path().join("blobs/sha256")).unwrap();
    let bytes: Vec<u8> = (0..size).map(|index| (index % 251) as u8).collect();
    std::fs::write(root.path().join(&relative), &bytes).unwrap();
    (root, vec![relative], bytes)
}

#[tokio::test]
async fn provision_leaves_resources_to_the_profile_when_unset() {
    let service = FakeService::start();
    service.route("POST", "/vms/create", 200, serde_json::to_value(vm(None)).unwrap());
    let request = ProvisionRequest {
        name: None,
        profile_id: "code".into(),
        ram_mb: None,
        cpus: None,
        persistent: false,
        env: None,
        from: None,
        networks: vec![],
    };
    assert_eq!(provision(&service.client, &request).await.unwrap().id, "vm-1");
    let body = service.find("POST", "/vms/create")[0].json();
    assert!(body.get("ram_mb").is_none() && body.get("cpus").is_none(), "{body}");
}

#[tokio::test]
async fn destroy_and_launch_report_what_the_service_refused() {
    let service = FakeService::start();
    service
        .once("DELETE", "/vms/vm-1/delete", 200, json!({"success": true}))
        .route("DELETE", "/vms/vm-1/delete", 404, json!({"error": "no such vm"}))
        .once("POST", "/vms/vm-1/exec", 200, exec(0, ""))
        .route("POST", "/vms/vm-1/exec", 200, exec(2, "setsid: not found"));
    destroy(&service.client, "vm-1").await.unwrap();
    let refused = destroy(&service.client, "vm-1").await.unwrap_err();
    assert!(format!("{refused:#}").contains("no such vm"), "{refused:#}");

    launch_detached(&service.client, &vm(None)).await.unwrap();
    let failed = launch_detached(&service.client, &vm(None)).await.unwrap_err();
    assert!(format!("{failed:#}").contains("setsid: not found"), "{failed:#}");
    let command = service.find("POST", "/vms/vm-1/exec")[0].json()["command"].clone();
    assert_eq!(command, json!(container::detached_launch_command()));
}

#[tokio::test]
async fn a_guest_that_is_not_ready_gets_no_upload() {
    let service = FakeService::start();
    service.route("POST", "/vms/vm-1/exec", 200, exec(1, "booting"));
    let (root, files, _) = layout(10);
    let args = image_args(&[]);
    let workload = Workload::of(&args, &[]).unwrap().unwrap();
    let blobs = Blobs {
        root: root.path(),
        files: &files,
    };
    let error = stage(&service.client, &vm(None), blobs, &workload).await.err().unwrap();
    assert!(
        format!("{error:#}").contains("readiness check failed: booting"),
        "{error:#}"
    );
    assert_eq!(service.calls(), ["POST /vms/vm-1/exec"]);
}

#[tokio::test]
async fn stage_uploads_verified_parts_below_the_body_limit_with_the_workload() {
    let service = FakeService::start();
    service.route("POST", "/vms/vm-1/exec", 200, exec(0, "")).route(
        "POST",
        "/vms/vm-1/files/content",
        200,
        json!({"success": true}),
    );
    // One and a half upload parts: the file API limit is never approached.
    let (root, files, bytes) = layout(3 * 1024 * 1024 / 2);
    let args = image_args(&[]);
    let env = vec!["MODE=cache".to_string()];
    let workload = Workload::of(&args, &env).unwrap().unwrap();
    let blobs = Blobs {
        root: root.path(),
        files: &files,
    };

    // No IPC socket: everything before attaching to the VM owner still ran.
    let error = stage(&service.client, &vm(None), blobs, &workload).await.err().unwrap();
    assert!(
        format!("{error:#}").contains("did not return the VM IPC socket"),
        "{error:#}"
    );

    let uploads = service.find("POST", "/vms/vm-1/files/content");
    let name = |index: usize| {
        let path = uploads[index].path.split("path=").nth(1).unwrap();
        urlencoding::decode(path).unwrap().into_owned()
    };
    let stage_path = |file: &str| format!("{}/{file}", container::STAGE);
    assert_eq!(
        (0..uploads.len()).map(name).collect::<Vec<_>>(),
        ["0-0", "0-1", "transfer.json", "options.json", "launch.py"].map(stage_path)
    );
    assert_eq!(uploads[0].body.len(), 1024 * 1024);
    assert_eq!([uploads[0].body.clone(), uploads[1].body.clone()].concat(), bytes);
    assert_eq!(
        uploads[2].json(),
        json!([{"path": "blobs/sha256/aa", "key": 0, "parts": 2, "sha256": format!("{:x}", Sha256::digest(&bytes))}])
    );
    assert_eq!(
        uploads[3].json(),
        json!({"args": ["redis-server", "--save"], "env": {"MODE": "cache"}})
    );
    assert_eq!(uploads[4].body, container::LAUNCHER);
}

#[tokio::test]
async fn a_refused_upload_stops_staging() {
    let service = FakeService::start();
    service.route("POST", "/vms/vm-1/exec", 200, exec(0, "")).route(
        "POST",
        "/vms/vm-1/files/content",
        200,
        json!({"success": false}),
    );
    let (root, files, _) = layout(10);
    let args = image_args(&[]);
    let workload = Workload::of(&args, &[]).unwrap().unwrap();
    let blobs = Blobs {
        root: root.path(),
        files: &files,
    };
    let error = stage(&service.client, &vm(None), blobs, &workload).await.err().unwrap();
    assert!(format!("{error:#}").contains("image upload was refused"), "{error:#}");
    assert_eq!(service.find("POST", "/vms/vm-1/files/content").len(), 1);
}

#[tokio::test]
async fn pull_refuses_what_it_cannot_name_before_any_network() {
    let args = ImageArgs {
        image: vec!["not a reference!".into()],
        ..ImageArgs::default()
    };
    let workload = Workload::of(&args, &[]).unwrap().unwrap();
    let error = pull(&workload).await.err().unwrap();
    assert!(format!("{error:#}").contains("--image expects"), "{error:#}");
}

#[test]
fn pull_refuses_a_registry_user_without_a_password_before_any_network() {
    let _lock = crate::lock_test_env();
    std::env::remove_var("CAPSEM_REGISTRY_PASSWORD");
    let args = ImageArgs {
        image: vec!["docker://redis:7".into()],
        registry_user: Some("me".into()),
        ..ImageArgs::default()
    };
    let workload = Workload::of(&args, &[]).unwrap().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let error = runtime.block_on(pull(&workload)).err().unwrap();
    assert!(format!("{error:#}").contains("CAPSEM_REGISTRY_PASSWORD"), "{error:#}");
}
