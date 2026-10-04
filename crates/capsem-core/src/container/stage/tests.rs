use super::*;

#[test]
fn stage_plan_writes_parts_then_the_files_the_launcher_reads() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("index.json"), b"{}").unwrap();
    std::fs::write(root.path().join("oci-layout"), b"").unwrap();
    let files = [PathBuf::from("index.json"), PathBuf::from("oci-layout")];
    let env = [("LANG".to_string(), "C".to_string())].into();
    let resources = super::super::workload_resources(2048, 2).unwrap();
    let surface = image_surface(root.path()).unwrap();
    assert_eq!(
        surface,
        DeclaredSurface::Terminal,
        "a layout naming no manifest declares nothing"
    );
    let plan = stage_plan(root.path(), &files, &["serve".to_string()], &env, resources, surface).unwrap();
    let names: Vec<&str> = plan.iter().map(|file| file.name.as_str()).collect();
    assert_eq!(names, ["0-0", "transfer.json", "options.json", "launch.py"]);
    assert!(matches!(&plan[0].content, StagedContent::File(path) if path == &root.path().join("index.json")));
    let StagedContent::Bytes(transfer) = &plan[1].content else {
        panic!("transfer.json is generated")
    };
    let transfer: serde_json::Value = serde_json::from_slice(transfer).unwrap();
    assert_eq!(transfer[1]["parts"], 0);
    let StagedContent::Bytes(options) = &plan[2].content else {
        panic!("options.json is generated")
    };
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(options).unwrap(),
        // The launcher mounts the VM workspace where the service says the
        // container sees it: one owner for that path, not two.
        // The host decides the workload's capabilities, syscall filter and
        // user-namespace map.
        serde_json::json!({
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
    assert!(matches!(&plan[3].content, StagedContent::Bytes(bytes) if bytes.as_slice() == LAUNCHER));
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
