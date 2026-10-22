use super::*;

#[test]
fn copy_missing_local_assets_materializes_hash_named_layout() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let install = dir.path().join("install");
    let arch_dir = source.join("arm64");
    std::fs::create_dir_all(&arch_dir).unwrap();

    let kernel = b"kernel-local";
    let initrd = b"initrd-local";
    let rootfs = b"rootfs-local";
    std::fs::write(arch_dir.join("vmlinuz"), kernel).unwrap();
    std::fs::write(arch_dir.join("initrd.img"), initrd).unwrap();
    std::fs::write(arch_dir.join("rootfs.erofs"), rootfs).unwrap();

    let manifest = ManifestV2::from_json(&format!(
        r#"{{
                "format": 2,
                "refresh_policy": "24h",
                "assets": {{
                    "current": "2030.0101.1",
                    "releases": {{
                        "2030.0101.1": {{
                            "date": "2030-01-01",
                            "deprecated": false,
                            "min_binary": "1.0.0",
                            "arches": {{
                                "arm64": {{
                                    "vmlinuz": {{ "hash": "{}", "size": {} }},
                                    "initrd.img": {{ "hash": "{}", "size": {} }},
                                    "rootfs.erofs": {{ "hash": "{}", "size": {} }}
                                }}
                            }}
                        }}
                    }}
                }},
                "binaries": {{
                    "current": "9.9.9",
                    "releases": {{
                        "9.9.9": {{
                            "date": "2030-01-01",
                            "deprecated": false,
                            "min_assets": "2030.0101.1"
                        }}
                    }}
                }}
            }}"#,
        capsem_crypto::blake3::hash(kernel).to_hex(),
        kernel.len(),
        capsem_crypto::blake3::hash(initrd).to_hex(),
        initrd.len(),
        capsem_crypto::blake3::hash(rootfs).to_hex(),
        rootfs.len(),
    ))
    .unwrap();

    let copied = copy_missing_local_assets(&manifest, "9.9.9", "arm64", &source, &install, |_| {}).unwrap();

    assert_eq!(copied.len(), 3);
    for (logical, bytes) in [
        ("vmlinuz", kernel.as_slice()),
        ("initrd.img", initrd.as_slice()),
        ("rootfs.erofs", rootfs.as_slice()),
    ] {
        let digest = capsem_crypto::blake3::hash(bytes).to_hex().to_string();
        let target = install.join("arm64").join(hash_filename(logical, &digest));
        assert_eq!(std::fs::read(&target).unwrap(), bytes);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o444);
        }
    }
}

#[test]
fn copy_missing_local_assets_rejects_hash_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let install = dir.path().join("install");
    std::fs::create_dir_all(source.join("arm64")).unwrap();
    std::fs::write(source.join("arm64").join("vmlinuz"), b"wrong").unwrap();
    std::fs::write(source.join("arm64").join("initrd.img"), b"initrd").unwrap();
    std::fs::write(source.join("arm64").join("rootfs.erofs"), b"rootfs").unwrap();
    let initrd_hash = capsem_crypto::blake3::hash(b"initrd").to_hex().to_string();
    let rootfs_hash = capsem_crypto::blake3::hash(b"rootfs").to_hex().to_string();

    let manifest = ManifestV2::from_json(
        &format!(
            r#"{{
                "format": 2,
                "refresh_policy": "24h",
                "assets": {{
                    "current": "2030.0101.1",
                    "releases": {{
                        "2030.0101.1": {{
                            "date": "2030-01-01",
                            "deprecated": false,
                            "min_binary": "1.0.0",
                            "arches": {{
                                "arm64": {{
                                    "vmlinuz": {{ "hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "size": 5 }},
                                    "initrd.img": {{ "hash": "{initrd_hash}", "size": 6 }},
                                    "rootfs.erofs": {{ "hash": "{rootfs_hash}", "size": 6 }}
                                }}
                            }}
                        }}
                    }}
                }},
                "binaries": {{
                    "current": "9.9.9",
                    "releases": {{
                        "9.9.9": {{
                            "date": "2030-01-01",
                            "deprecated": false,
                            "min_assets": "2030.0101.1"
                        }}
                    }}
                }}
            }}"#,
        ),
    )
    .unwrap();

    let err = copy_missing_local_assets(&manifest, "9.9.9", "arm64", &source, &install, |_| {})
        .expect_err("wrong bytes must not be installed");
    assert!(err.to_string().contains("hash mismatch"), "{err:#}");
    assert!(!install.join("arm64").join("vmlinuz-aaaaaaaaaaaaaaaa").exists());
}

#[tokio::test]
async fn download_missing_assets_skips_direct_arch_dev_layout() {
    let dir = tempfile::tempdir().unwrap();
    let base_dir = dir.path().join("arm64");
    std::fs::create_dir(&base_dir).unwrap();
    let files = [
        ("vmlinuz", b"kernel".as_slice()),
        ("initrd.img", b"initrd".as_slice()),
        ("rootfs.erofs", b"rootfs".as_slice()),
    ];
    let mut assets = std::collections::HashMap::new();
    for (name, bytes) in files {
        let hash = capsem_crypto::blake3::hash(bytes).to_hex().to_string();
        assets.insert(
            name.to_string(),
            AssetEntry {
                hash,
                sha256: String::new(),
                size: bytes.len() as u64,
            },
        );
    }
    let manifest = ManifestV2 {
        format: 2,
        refresh_policy: "24h".to_string(),
        asset_base: None,
        assets: AssetsSection {
            current: "2030.0101.1".to_string(),
            releases: [(
                "2030.0101.1".to_string(),
                AssetRelease {
                    date: "2030-01-01".to_string(),
                    deprecated: false,
                    deprecated_date: None,
                    min_binary: "1.0.0".to_string(),
                    arches: [("arm64".to_string(), assets)].into(),
                },
            )]
            .into(),
        },
        binaries: BinariesSection {
            current: "9.9.9".to_string(),
            releases: [(
                "9.9.9".to_string(),
                BinaryRelease {
                    date: "2030-01-01".to_string(),
                    deprecated: false,
                    deprecated_date: None,
                    min_assets: "2030.0101.1".to_string(),
                    version: String::new(),
                    files: Vec::new(),
                },
            )]
            .into(),
        },
    };
    for (name, entry) in &manifest.assets.releases["2030.0101.1"].arches["arm64"] {
        let hname = hash_filename(name, &entry.hash);
        let bytes = match name.as_str() {
            "vmlinuz" => b"kernel".as_slice(),
            "initrd.img" => b"initrd".as_slice(),
            "rootfs.erofs" => b"rootfs".as_slice(),
            _ => unreachable!(),
        };
        std::fs::write(base_dir.join(hname), bytes).unwrap();
    }

    let downloaded = download_missing_assets(&manifest, "9.9.9", "arm64", &base_dir, |_| {})
        .await
        .expect("direct arch layout should not try to download");

    assert!(downloaded.is_empty());
}

// CAPSEM_ASSET_BASE_URL override is exercised end-to-end by the Python
// integration test in tests/capsem_install/test_asset_download.py against
// a real local HTTP server. We deliberately don't unit-test it here:
// env mutation is process-wide and races with other tests in this binary.

#[test]
fn copy_missing_local_assets_hydrates_every_compatible_release() {
    // A manifest can carry several compatible asset releases, and `current`
    // names only one of them. Hydration resolved that one and copied its
    // assets alone, so installing a manifest whose releases pin different
    // kernels left the other kernel absent: a fresh install that could not
    // boot what it reported ready.
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let install = dir.path().join("install");
    let arch_dir = source.join("arm64");
    std::fs::create_dir_all(&arch_dir).unwrap();

    // Two releases, distinct kernels, one shared rootfs.
    let code_kernel = b"kernel-for-code";
    let cowork_kernel = b"kernel-for-co-work";
    let rootfs = b"rootfs-shared";
    std::fs::write(arch_dir.join("vmlinuz"), code_kernel).unwrap();
    std::fs::write(arch_dir.join("vmlinuz-co-work"), cowork_kernel).unwrap();
    std::fs::write(arch_dir.join("rootfs.erofs"), rootfs).unwrap();

    let entry = |bytes: &[u8]| {
        format!(
            r#"{{ "hash": "{}", "size": {} }}"#,
            capsem_crypto::blake3::hash(bytes).to_hex(),
            bytes.len()
        )
    };
    let manifest = ManifestV2::from_json(&format!(
        r#"{{
            "format": 2,
            "refresh_policy": "24h",
            "assets": {{
                "current": "2030.0101.1",
                "releases": {{
                    "2030.0101.1": {{
                        "min_binary": "1.0.0",
                        "arches": {{ "arm64": {{
                            "vmlinuz": {},
                            "rootfs.erofs": {}
                        }} }}
                    }},
                    "2030.0101.1+co-work": {{
                        "min_binary": "1.0.0",
                        "arches": {{ "arm64": {{
                            "vmlinuz-co-work": {},
                            "rootfs.erofs": {}
                        }} }}
                    }}
                }}
            }},
            "binaries": {{
                "current": "9.9.9",
                "releases": {{ "9.9.9": {{ "min_assets": "2030.0101.1" }} }}
            }}
        }}"#,
        entry(code_kernel),
        entry(rootfs),
        entry(cowork_kernel),
        entry(rootfs),
    ))
    .unwrap();

    copy_missing_local_assets(&manifest, "9.9.9", "arm64", &source, &install, |_| {}).unwrap();

    for (logical, bytes) in [
        ("vmlinuz", code_kernel.as_slice()),
        ("vmlinuz-co-work", cowork_kernel.as_slice()),
        ("rootfs.erofs", rootfs.as_slice()),
    ] {
        let digest = capsem_crypto::blake3::hash(bytes).to_hex().to_string();
        let target = install.join("arm64").join(hash_filename(logical, &digest));
        assert!(
            target.exists(),
            "{logical} was never hydrated: a VM pinning it cannot boot"
        );
        assert_eq!(std::fs::read(&target).unwrap(), bytes);
    }
}

#[test]
fn materializing_keeps_both_releases_images_when_they_share_a_logical_name() {
    // Two releases each ship a `vmlinuz`. They are different bytes, they land
    // under different hash-named files, and both have to exist -- so the set
    // of things to materialize is keyed by name *and* hash. Keyed by name
    // alone this quietly keeps one, which is the missing-kernel install all
    // over again, and no arch/name assertion would notice.
    let code = b"kernel-for-code";
    let cowork = b"kernel-for-co-work";
    let entry = |bytes: &[u8]| {
        format!(
            r#"{{ "hash": "{}", "size": {} }}"#,
            capsem_crypto::blake3::hash(bytes).to_hex(),
            bytes.len()
        )
    };
    let manifest = ManifestV2::from_json(&format!(
        r#"{{
            "format": 2,
            "refresh_policy": "24h",
            "assets": {{
                "current": "2030.0101.1",
                "releases": {{
                    "2030.0101.1": {{ "arches": {{ "arm64": {{ "vmlinuz": {} }} }} }},
                    "2030.0101.1+co-work": {{ "arches": {{ "arm64": {{ "vmlinuz": {} }} }} }}
                }}
            }},
            "binaries": {{ "current": "9.9.9", "releases": {{ "9.9.9": {{}} }} }}
        }}"#,
        entry(code),
        entry(cowork),
    ))
    .unwrap();

    let wanted = arch_assets_to_materialize(&manifest, "9.9.9", "arm64").unwrap();
    let hashes: Vec<&str> = wanted.iter().map(|(_, _, entry)| entry.hash.as_str()).collect();

    assert_eq!(wanted.len(), 2, "one release's kernel was dropped: {hashes:?}");
    for bytes in [code.as_slice(), cowork.as_slice()] {
        assert!(hashes.contains(&capsem_crypto::blake3::hash(bytes).to_hex().to_string().as_str()));
    }
    // Each carries the release it came from, because that is what names its URL.
    let versions: Vec<&str> = wanted.iter().map(|(version, _, _)| *version).collect();
    assert!(versions.contains(&"2030.0101.1") && versions.contains(&"2030.0101.1+co-work"));
}

#[test]
fn materializing_refuses_an_arch_no_compatible_release_builds() {
    // Skipping a release that lacks the arch must not become "nothing to do":
    // an install that materializes zero assets and reports success is the
    // failure mode this whole path exists to prevent.
    let manifest = ManifestV2::from_json(
        r#"{
            "format": 2,
            "refresh_policy": "24h",
            "assets": {
                "current": "2030.0101.1",
                "releases": {
                    "2030.0101.1": { "arches": { "arm64": { "vmlinuz": { "hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "size": 1 } } } }
                }
            },
            "binaries": { "current": "9.9.9", "releases": { "9.9.9": {} } }
        }"#,
    )
    .unwrap();

    let error = arch_assets_to_materialize(&manifest, "9.9.9", "x86_64").unwrap_err();
    assert!(error.to_string().contains("x86_64"), "unhelpful error: {error}");
}

// -----------------------------------------------------------------------
// Download size bound
// -----------------------------------------------------------------------
//
// The manifest size was used for progress only; the stream was written to
// disk until EOF and only then hashed. The origin is manifest-controlled, so
// a hostile or broken server could fill the disk before the hash check ran.

/// Serve `arm64-<name>` requests from `bodies`, one HTTP/1.1 response per
/// connection. Returns an asset base URL that `asset_download_url_with_base`
/// expands under it.
async fn serve_asset_bodies(bodies: std::collections::HashMap<&'static str, Vec<u8>>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = sock.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                seen.extend_from_slice(&buf[..n]);
            }
            let request = String::from_utf8_lossy(&seen);
            let path = request.split_whitespace().nth(1).unwrap_or("");
            let name = path.rsplit("arm64-").next().unwrap_or("");
            let body = bodies.get(name).cloned().unwrap_or_default();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(head.as_bytes()).await.unwrap();
            sock.write_all(&body).await.unwrap();
            let _ = sock.shutdown().await;
        }
    });
    format!("http://{addr}/assets/{{asset_version}}")
}

const DECLARED_ASSETS: [(&str, &[u8]); 3] = [
    ("initrd.img", b"initrd"),
    ("rootfs.erofs", b"rootfs"),
    ("vmlinuz", b"kernel"),
];

#[tokio::test]
async fn service_hydration_uses_the_packaged_local_manifest_source() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source with spaces");
    let installed = dir.path().join("installed");
    std::fs::create_dir_all(source.join("arm64")).unwrap();
    std::fs::create_dir_all(&installed).unwrap();
    let mut manifest = declared_manifest(String::new());
    manifest.asset_base = None;
    let payload = serde_json::to_vec(&manifest).unwrap();
    std::fs::write(source.join("manifest.json"), &payload).unwrap();
    std::fs::write(installed.join("manifest.json"), &payload).unwrap();
    std::fs::write(
        installed.join("manifest-metadata.json"),
        serde_json::to_vec(&serde_json::json!({
            "manifest_url": reqwest::Url::from_file_path(source.join("manifest.json")).unwrap().as_str()
        }))
        .unwrap(),
    )
    .unwrap();
    for (name, bytes) in DECLARED_ASSETS {
        std::fs::write(source.join("arm64").join(name), bytes).unwrap();
    }
    let hydrate = || download_missing_assets(&manifest, "9.9.9", "arm64", &installed, |_| {});
    let paths = tokio::time::timeout(std::time::Duration::from_secs(2), hydrate())
        .await
        .expect("local hydration must not contact the public release server")
        .expect("the service must honor the package's file:// source");
    assert_eq!(paths.len(), 3);
    for (name, bytes) in DECLARED_ASSETS {
        let hash = capsem_crypto::blake3::hash(bytes).to_hex().to_string();
        assert_eq!(
            std::fs::read(installed.join("arm64").join(hash_filename(name, &hash))).unwrap(),
            bytes
        );
    }
    assert!(hydrate().await.unwrap().is_empty(), "verified bytes are reused");
    // An interrupted local install remains repairable from that same source.
    std::fs::remove_file(&paths[0]).unwrap();
    assert_eq!(hydrate().await.unwrap().len(), 1);
}

#[test]
fn local_manifest_metadata_never_falls_back_on_invalid_or_missing_sources() {
    let dir = tempfile::tempdir().unwrap();
    for source in [
        "file:///definitely-missing-capsem-runtime/manifest.json",
        "file:relative/manifest.json",
        "/tmp/manifest.json",
        "ftp://example.invalid/manifest.json",
    ] {
        std::fs::write(
            dir.path().join("manifest-metadata.json"),
            serde_json::json!({"manifest_url":source}).to_string(),
        )
        .unwrap();
        assert!(local_manifest_asset_source(dir.path()).is_err(), "{source}");
    }
}

fn declared_manifest(asset_base: String) -> ManifestV2 {
    let mut assets = std::collections::HashMap::new();
    for (name, bytes) in DECLARED_ASSETS {
        assets.insert(
            name.to_string(),
            AssetEntry {
                hash: capsem_crypto::blake3::hash(bytes).to_hex().to_string(),
                sha256: String::new(),
                size: bytes.len() as u64,
            },
        );
    }
    ManifestV2 {
        format: 2,
        refresh_policy: "24h".to_string(),
        asset_base: Some(asset_base),
        assets: AssetsSection {
            current: "2030.0101.1".to_string(),
            releases: [(
                "2030.0101.1".to_string(),
                AssetRelease {
                    date: "2030-01-01".to_string(),
                    deprecated: false,
                    deprecated_date: None,
                    min_binary: "1.0.0".to_string(),
                    arches: [("arm64".to_string(), assets)].into(),
                },
            )]
            .into(),
        },
        binaries: BinariesSection {
            current: "9.9.9".to_string(),
            releases: [(
                "9.9.9".to_string(),
                BinaryRelease {
                    date: "2030-01-01".to_string(),
                    deprecated: false,
                    deprecated_date: None,
                    min_assets: "2030.0101.1".to_string(),
                    version: String::new(),
                    files: Vec::new(),
                },
            )]
            .into(),
        },
    }
}

#[tokio::test]
async fn download_refuses_a_body_longer_than_the_manifest_size() {
    let dir = tempfile::tempdir().unwrap();
    let mut bodies: std::collections::HashMap<&'static str, Vec<u8>> = DECLARED_ASSETS
        .iter()
        .map(|(name, bytes)| (*name, bytes.to_vec()))
        .collect();
    // Assets download in name order; the first one lies about its length.
    bodies.insert("initrd.img", b"initrd-but-much-longer-than-declared".to_vec());
    let manifest = declared_manifest(serve_asset_bodies(bodies).await);

    let err = download_missing_assets(&manifest, "9.9.9", "arm64", dir.path(), |_| {})
        .await
        .expect_err("a body past the manifest size must be refused");
    assert!(err.to_string().contains("more than the manifest size"), "{err:#}");

    let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("arm64"))
        .map(|entries| entries.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(leftovers.is_empty(), "no partial download may remain: {leftovers:?}");
}

#[tokio::test]
async fn download_accepts_bodies_that_match_the_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let bodies = DECLARED_ASSETS
        .iter()
        .map(|(name, bytes)| (*name, bytes.to_vec()))
        .collect();
    let manifest = declared_manifest(serve_asset_bodies(bodies).await);

    let mut downloaded = download_missing_assets(&manifest, "9.9.9", "arm64", dir.path(), |_| {})
        .await
        .expect("exact bodies download");
    downloaded.sort();
    assert_eq!(downloaded.len(), 3);
    assert_eq!(std::fs::read(&downloaded[2]).unwrap(), b"kernel");
}

#[tokio::test]
async fn stalled_runtime_body_times_out_cleans_up_and_can_be_retried() {
    use std::{sync::Arc, time::Duration};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let partial = Arc::new(tokio::sync::Notify::new());
    tokio::spawn(async move {
        let mut first = true;
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let stall = std::mem::replace(&mut first, false);
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|chunk| chunk == b"\r\n\r\n") {
                    let size = socket.read(&mut buffer).await.unwrap();
                    if size == 0 {
                        return;
                    }
                    request.extend_from_slice(&buffer[..size]);
                }
                let request = String::from_utf8_lossy(&request);
                let path = request.split_whitespace().nth(1).unwrap();
                let (_, body) = DECLARED_ASSETS.iter().find(|(name, _)| path.ends_with(name)).unwrap();
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                socket.write_all(if stall { &body[..1] } else { body }).await.unwrap();
                if stall {
                    // Keep the connection open without another byte, as a stalled origin does.
                    std::future::pending::<()>().await;
                }
            });
        }
    });
    let manifest = declared_manifest(format!("http://{address}/assets/{{asset_version}}"));
    let directory = tempfile::tempdir().unwrap();
    let task_manifest = manifest.clone();
    let path = directory.path().to_path_buf();
    let observed = Arc::clone(&partial);
    let download = tokio::spawn(async move {
        download_missing_assets(&task_manifest, "9.9.9", "arm64", &path, move |progress| {
            if progress.bytes_done == 1 {
                observed.notify_one();
            }
        })
        .await
    });
    tokio::time::timeout(Duration::from_secs(3), partial.notified())
        .await
        .expect("one real body byte is written");
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    let result = tokio::time::timeout(Duration::from_secs(1), download)
        .await
        .expect("stalled runtime read must terminate")
        .unwrap();
    tokio::time::resume();
    let error = result.expect_err("a stalled body cannot become a verified asset");
    assert!(
        error.chain().any(|cause| cause
            .downcast_ref::<reqwest::Error>()
            .is_some_and(reqwest::Error::is_timeout)),
        "{error:#}"
    );
    assert_eq!(
        std::fs::read_dir(directory.path().join("arm64")).unwrap().count(),
        0,
        "the partial file must be removed"
    );
    let downloaded = download_missing_assets(&manifest, "9.9.9", "arm64", directory.path(), |_| {})
        .await
        .unwrap();
    assert_eq!(downloaded.len(), 3);
    for (name, body) in DECLARED_ASSETS {
        let hash = capsem_crypto::blake3::hash(body).to_hex().to_string();
        let path = directory.path().join("arm64").join(hash_filename(name, &hash));
        assert_eq!(std::fs::read(path).unwrap(), body);
    }
}

#[tokio::test]
async fn runtime_download_deadline_is_shared_across_files() {
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::Duration,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let bytes = vec![b'r'; 31];
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let next_byte = Arc::new(tokio::sync::Notify::new());
    let fast = Arc::new(AtomicBool::new(false));
    let server_next = Arc::clone(&next_byte);
    let server_fast = Arc::clone(&fast);
    let body = bytes.clone();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket.set_nodelay(true).unwrap();
            let next = Arc::clone(&server_next);
            let fast = Arc::clone(&server_fast);
            let body = body.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|chunk| chunk == b"\r\n\r\n") {
                    let size = socket.read(&mut buffer).await.unwrap();
                    if size == 0 {
                        return;
                    }
                    request.extend_from_slice(&buffer[..size]);
                }
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                if fast.load(Ordering::Relaxed) {
                    socket.write_all(&body).await.unwrap();
                } else {
                    for (index, byte) in body.into_iter().enumerate() {
                        if index > 0 {
                            next.notified().await;
                        }
                        if socket.write_all(&[byte]).await.is_err() {
                            return;
                        }
                    }
                }
            });
        }
    });
    let mut manifest = declared_manifest(format!("http://{address}/assets/{{asset_version}}"));
    for asset in manifest
        .assets
        .releases
        .get_mut("2030.0101.1")
        .unwrap()
        .arches
        .get_mut("arm64")
        .unwrap()
        .values_mut()
    {
        asset.size = bytes.len() as u64;
        asset.hash = capsem_crypto::blake3::hash(&bytes).to_hex().to_string();
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_path_buf();
    let selected = manifest.clone();
    let (updates, mut progress) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        download_missing_assets(&selected, "9.9.9", "arm64", &path, move |value| {
            updates.send(value).unwrap();
        })
        .await
    });
    // Read an actual byte before pausing, so virtual time cannot outrun TCP setup.
    let first = tokio::time::timeout(Duration::from_secs(3), progress.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.bytes_done, 1);
    // Keep the paused clock from auto-advancing while real loopback I/O settles.
    let clock = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    tokio::time::pause();
    let wall_deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut done = first.done;
    let mut elapsed = 0;
    while !task.is_finished() {
        if !done {
            tokio::time::advance(Duration::from_secs(20)).await;
            elapsed += 20;
            next_byte.notify_one();
        }
        loop {
            if task.is_finished() {
                break;
            }
            if let Ok(update) = progress.try_recv() {
                // The final byte arrives before the separate `done` update.
                // Do not advance across that gap: the next file can already
                // be opening its real TCP connection (with a 10-second budget).
                done = update.done || update.bytes_total.is_some_and(|total| update.bytes_done >= total);
                break;
            }
            assert!(
                std::time::Instant::now() < wall_deadline,
                "the real loopback transfer stopped making progress"
            );
            tokio::task::yield_now().await;
        }
    }
    tokio::time::resume();
    clock.abort();
    let error = task
        .await
        .unwrap()
        .expect_err("three moving files cannot reset the runtime deadline");
    assert!(
        error.chain().any(|cause| cause
            .downcast_ref::<reqwest::Error>()
            .is_some_and(reqwest::Error::is_timeout)),
        "{error:#}"
    );
    assert_eq!(elapsed, 900, "the full runtime has one fifteen-minute network budget");
    let first = directory.path().join("arm64").join(hash_filename(
        "initrd.img",
        capsem_crypto::blake3::hash(&bytes).to_hex().as_ref(),
    ));
    assert_eq!(std::fs::read(first).unwrap(), bytes);
    assert_eq!(
        std::fs::read_dir(directory.path().join("arm64")).unwrap().count(),
        1,
        "keep verified files but remove the timed-out partial"
    );
    fast.store(true, Ordering::Relaxed);
    assert_eq!(
        download_missing_assets(&manifest, "9.9.9", "arm64", directory.path(), |_| {})
            .await
            .unwrap()
            .len(),
        2
    );
}
