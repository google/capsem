use super::*;

const MEDIA: &str = "application/vnd.capsem.package-inventory.v1+json";

fn artifact(registry: &Registry, change: impl FnOnce(&mut Value, &mut Value)) -> (String, String) {
    let image = format!("{}/team/image@{}", registry.address, registry.source_digest);
    let subject_size =
        registry.blobs.lock().unwrap()[&format!("/v2/team/image/manifests/{}", registry.source_digest)].len();
    let mut document = json!({"schema":"capsem.package_inventory.v1", "subject":image,
        "architecture":"arm64", "packages":[{"name":"git","version":"2.39.5","architecture":"arm64"}],
        "tools":[{"name":"gh","version":"gh version 2.23.0"}]});
    let mut manifest = json!({"schemaVersion":2,"mediaType":OCI_IMAGE_MEDIA_TYPE,"artifactType":MEDIA,
        "config":descriptor("application/vnd.oci.empty.v1+json", b"{}"),
        "subject":{"mediaType":OCI_IMAGE_MEDIA_TYPE,"digest":registry.source_digest,"size":subject_size},
        "layers":[{}]});
    change(&mut manifest, &mut document);
    let bytes = serde_json::to_vec(&document).unwrap();
    if manifest["layers"][0].as_object().unwrap().is_empty() {
        manifest["layers"][0] = descriptor(MEDIA, &bytes);
    }
    let encoded = serde_json::to_vec(&manifest).unwrap();
    let pin = digest(&encoded);
    let mut blobs = registry.blobs.lock().unwrap();
    blobs.insert(format!("/v2/team/image/manifests/{pin}"), encoded);
    blobs.insert(format!("/v2/team/image/blobs/{}", digest(&bytes)), bytes);
    drop(blobs);
    (image, format!("{}/team/image@{pin}", registry.address))
}

#[tokio::test]
async fn reads_inventory_without_downloading_or_unpacking_image_layers() {
    let registry = Registry::start(|_, _| {}).await;
    let (image, artifact) = artifact(&registry, |_, _| {});
    let parent = tempfile::tempdir().unwrap();
    let inventory = registry
        .puller()
        .fetch_inventory(&image, &artifact, parent.path())
        .await
        .unwrap();
    assert_eq!(inventory.packages[0].name, "git");
    assert_eq!(inventory.tools[0].name, "gh");
    assert_eq!(inventory.subject, image);
    let layer_path = format!("/v2/team/image/blobs/{}", digest(&registry.layer));
    assert!(!registry
        .requests
        .lock()
        .unwrap()
        .iter()
        .any(|request| request.path == layer_path));
}

#[tokio::test]
async fn refuses_wrong_subject_platform_media_size_urls_and_schema() {
    for case in 0..15 {
        let registry = Registry::start(|_, _| {}).await;
        let (image, artifact) = artifact(&registry, |manifest, document| match case {
            0 => manifest["subject"]["digest"] = json!(format!("sha256:{}", "0".repeat(64))),
            1 => document["architecture"] = json!("amd64"),
            2 => manifest["artifactType"] = json!("text/html"),
            3 => {
                manifest["layers"][0] =
                    json!({"mediaType":MEDIA,"digest":format!("sha256:{}","0".repeat(64)),"size":4194305})
            }
            4 => {
                manifest["layers"][0] = json!({"mediaType":MEDIA,"digest":format!("sha256:{}","0".repeat(64)),"size":1,"urls":["http://127.0.0.1/private"]})
            }
            5 => document["subject"] = json!("registry.example/other@sha256:bad"),
            6 => document["schema"] = json!("unsupported"),
            7 => manifest["layers"][0] = descriptor("text/html", b"wrong"),
            8 => manifest["subject"]["size"] = json!(1),
            9 => manifest["schemaVersion"] = json!(1),
            10 => manifest["config"]["digest"] = json!(format!("sha256:{}", "0".repeat(64))),
            11 => document["packages"] = json!([]),
            12 => document["tools"][0]["version"] = json!("bad\nversion"),
            13 => document["packages"][0]["architecture"] = json!("unknown"),
            _ => manifest["config"]["urls"] = json!(["https://example.test/foreign"]),
        });
        let parent = tempfile::tempdir().unwrap();
        assert!(
            registry
                .puller()
                .fetch_inventory(&image, &artifact, parent.path())
                .await
                .is_err(),
            "case {case}"
        );
    }
}

#[tokio::test]
async fn rejects_corrupt_inventory_bytes_and_foreign_repository_before_fetch() {
    let registry = Registry::start(|_, _| {}).await;
    let (image, artifact) = artifact(&registry, |_, _| {});
    {
        let mut blobs = registry.blobs.lock().unwrap();
        let (_, bytes) = blobs
            .iter_mut()
            .find(|(key, bytes)| {
                key.contains("/blobs/")
                    && serde_json::from_slice::<Value>(bytes)
                        .ok()
                        .is_some_and(|v| v["schema"] == "capsem.package_inventory.v1")
            })
            .unwrap();
        bytes[0] = b'!';
        drop(blobs);
    }
    let parent = tempfile::tempdir().unwrap();
    let error = registry
        .puller()
        .fetch_inventory(&image, &artifact, parent.path())
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("digest"));
    let before = registry.requests.lock().unwrap().len();
    let foreign = artifact.replace("/team/image@", "/different/repository@");
    assert!(registry
        .puller()
        .fetch_inventory(&image, &foreign, parent.path())
        .await
        .is_err());
    assert_eq!(registry.requests.lock().unwrap().len(), before);
}
