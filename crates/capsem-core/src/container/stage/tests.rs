use super::*;

const MANIFEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[test]
fn published_root_stage_metadata_is_typed_and_keeps_the_single_stage_path() {
    let rootfs = RootfsFile {
        digest: MANIFEST,
        size: 12345,
    };
    let image = StageImage {
        manifest: MANIFEST,
        rootfs: Some(rootfs),
    };
    let resources = super::super::workload_resources(2048, 2).unwrap();
    let plan = stage_plan(image, &[], &BTreeMap::new(), resources, DeclaredSurface::Terminal).unwrap();
    assert_eq!(
        plan.iter().map(|file| file.name.as_str()).collect::<Vec<_>>(),
        ["options.json", "launch.py"]
    );
    let options: serde_json::Value = serde_json::from_slice(&plan[0].bytes).unwrap();
    assert_eq!(options["manifest"], MANIFEST);
    assert_eq!(
        options["rootfs"],
        serde_json::json!({"digest": MANIFEST, "size": 12345})
    );
    assert_eq!(
        options["id_map"],
        serde_json::json!({"containerID": 0, "hostID": 100000, "size": 65536})
    );

    let owned = MANIFEST.to_owned();
    let without = stage_plan(&owned, &[], &BTreeMap::new(), resources, DeclaredSurface::Terminal).unwrap();
    let options: serde_json::Value = serde_json::from_slice(&without[0].bytes).unwrap();
    assert!(
        options.get("rootfs").is_none(),
        "old callers retain exactly the universal-path options"
    );
}

#[test]
fn published_root_stage_metadata_rejects_bad_digests_and_out_of_bound_sizes() {
    let resources = super::super::workload_resources(2048, 2).unwrap();
    for (digest, size) in [("sha256:../bad", 1), (MANIFEST, 0), (MANIFEST, 4 * 1024u64.pow(3) + 1)] {
        let image = StageImage {
            manifest: MANIFEST,
            rootfs: Some(RootfsFile { digest, size }),
        };
        assert!(stage_plan(image, &[], &BTreeMap::new(), resources, DeclaredSurface::Terminal).is_err());
    }
}

/// The stage holds only the two control files: the image is in the share,
/// named here by its manifest digest, and no layer byte is staged.
#[test]
fn stage_plan_writes_only_the_options_and_the_launcher() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("index.json"), b"{}").unwrap();
    let env = [("LANG".to_string(), "C".to_string())].into();
    let resources = super::super::workload_resources(2048, 2).unwrap();
    let surface = image_surface(root.path()).unwrap();
    assert_eq!(
        surface,
        DeclaredSurface::Terminal,
        "a layout naming no manifest declares nothing"
    );
    let plan = stage_plan(MANIFEST, &["serve".to_string()], &env, resources, surface).unwrap();
    let names: Vec<&str> = plan.iter().map(|file| file.name.as_str()).collect();
    assert_eq!(names, ["options.json", "launch.py"]);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&plan[0].bytes).unwrap(),
        // The launcher mounts the VM workspace where the service says the
        // container sees it: one owner for that path, not two.
        // The host decides the workload's capabilities, syscall filter and
        // user-namespace map.
        serde_json::json!({
            "manifest": MANIFEST,
            "args": ["serve"],
            "env": {"LANG": "C"},
            "workspace": super::super::CONTAINER_WORKSPACE,
            "capabilities": super::super::seccomp::WORKLOAD_CAPABILITIES,
            "seccomp": super::super::seccomp::workload_seccomp(super::oci_architecture().unwrap(), super::super::seccomp::Surface::Terminal).unwrap(),
            "id_map": {"containerID": 0, "hostID": 100000, "size": 65536},
            "resources": {"memory_bytes": 1664u64 * 1024 * 1024, "cpu_millis": 1750, "pids": 4096},
            "surface": "terminal",
        })
    );
    assert_eq!(super::super::CONTAINER_WORKSPACE, "/workspace");
    assert_eq!(plan[1].bytes, LAUNCHER);
    let resources = super::super::workload_resources(2048, 2).unwrap();
    assert!(
        stage_plan("sha256:short", &[], &BTreeMap::new(), resources, surface).is_err(),
        "the stage only ever names a valid manifest digest"
    );
}

/// A pulled layout: index -> manifest -> config and one layer, every blob
/// listed the way the puller lists its verified files.
fn layout(root: &Path) -> Vec<PathBuf> {
    let blobs = root.join("blobs/sha256");
    std::fs::create_dir_all(&blobs).unwrap();
    let hex = |byte: char| byte.to_string().repeat(64);
    let manifest = serde_json::json!({
        "config": {"digest": format!("sha256:{}", hex('c'))},
        "layers": [{"digest": format!("sha256:{}", hex('d'))}],
    });
    std::fs::write(blobs.join(hex('a')), serde_json::to_vec(&manifest).unwrap()).unwrap();
    std::fs::write(
        root.join("index.json"),
        serde_json::to_vec(&serde_json::json!({"manifests": [{"digest": MANIFEST}]})).unwrap(),
    )
    .unwrap();
    let mut files = vec![PathBuf::from("index.json"), PathBuf::from("oci-layout")];
    for byte in ['a', 'c', 'd'] {
        files.push(PathBuf::from("blobs/sha256").join(hex(byte)));
    }
    files
}

#[test]
fn image_blobs_are_the_verified_blobs_and_the_manifest_the_index_names() {
    let root = tempfile::tempdir().unwrap();
    let files = layout(root.path());
    let image = image_blobs(root.path(), &files).unwrap();
    assert_eq!(image.manifest, MANIFEST);
    assert_eq!(image.blobs, ["a", "c", "d"].map(|byte| byte.repeat(64)));
}

#[test]
fn a_manifest_naming_a_blob_the_pull_did_not_verify_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let mut files = layout(root.path());
    let layer = files.pop().unwrap();
    let error = image_blobs(root.path(), &files).unwrap_err().to_string();
    assert!(error.contains("did not verify"), "{error}");
    files.push(layer);
    files.push(PathBuf::from("blobs/sha256/not-a-digest"));
    assert!(image_blobs(root.path(), &files).is_err());
    std::fs::write(root.path().join("index.json"), b"{\"manifests\":[]}").unwrap();
    assert!(
        image_blobs(root.path(), &files[..files.len() - 1]).is_err(),
        "no manifest, no image"
    );
}

#[test]
fn oci_architecture_names_the_host_the_way_registries_do() {
    let expected = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        _ => return assert!(oci_architecture().is_err()),
    };
    assert_eq!(oci_architecture().unwrap(), expected);
    // The catalog names the same architecture the registry does.
    assert_eq!(catalog_architecture().unwrap().as_str(), expected);
}

fn labels(pairs: &[(&str, serde_json::Value)]) -> serde_json::Map<String, serde_json::Value> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn xpra_on(port: &str) -> serde_json::Map<String, serde_json::Value> {
    labels(&[(SURFACE_LABEL, "xpra".into()), (SURFACE_PORT_LABEL, port.into())])
}

#[test]
fn an_xpra_image_declares_exactly_its_one_surface_port() {
    let declared = DeclaredSurface::from_labels(&xpra_on("14500")).unwrap();
    assert_eq!(declared, DeclaredSurface::Xpra { port: 14500 });
    assert_eq!(declared.seccomp(), super::super::seccomp::Surface::Xpra);
    assert_eq!(
        DeclaredSurface::from_labels(&xpra_on("1")).unwrap(),
        DeclaredSurface::Xpra { port: 1 }
    );
    assert_eq!(
        DeclaredSurface::from_labels(&xpra_on("65535")).unwrap(),
        DeclaredSurface::Xpra { port: 65535 }
    );
}

#[test]
fn a_terminal_image_declares_no_port() {
    assert_eq!(
        DeclaredSurface::from_labels(&labels(&[])).unwrap(),
        DeclaredSurface::Terminal
    );
    let terminal = labels(&[(SURFACE_LABEL, "terminal".into())]);
    assert_eq!(
        DeclaredSurface::from_labels(&terminal).unwrap(),
        DeclaredSurface::Terminal
    );
    assert_eq!(
        DeclaredSurface::Terminal.seccomp(),
        super::super::seccomp::Surface::Terminal
    );
}

/// An image cannot widen what it is granted: anything but one plain decimal
/// port in 1..=65535 on an xpra image refuses the image.
#[test]
fn a_surface_port_that_is_not_exactly_one_port_refuses_the_image() {
    for port in [
        "",
        "0",
        "00000",
        "65536",
        "70000",
        "+14500",
        "-1",
        " 14500",
        "14500 ",
        "14500,14501",
        "14500-14501",
        "0x3894",
        "1e4",
        "145000",
    ] {
        assert!(
            DeclaredSurface::from_labels(&xpra_on(port)).is_err(),
            "{port:?} was accepted"
        );
    }
    let numeric = labels(&[(SURFACE_LABEL, "xpra".into()), (SURFACE_PORT_LABEL, 14500.into())]);
    assert!(DeclaredSurface::from_labels(&numeric).is_err(), "labels are strings");
    let missing = labels(&[(SURFACE_LABEL, "xpra".into())]);
    assert!(DeclaredSurface::from_labels(&missing).is_err());
    let on_terminal = labels(&[(SURFACE_PORT_LABEL, "8080".into())]);
    assert!(
        DeclaredSurface::from_labels(&on_terminal).is_err(),
        "a terminal image has no surface port"
    );
    let unknown = labels(&[(SURFACE_LABEL, "vnc".into()), (SURFACE_PORT_LABEL, "5900".into())]);
    assert!(DeclaredSurface::from_labels(&unknown).is_err());
}
