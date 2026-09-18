use super::*;

#[test]
fn stage_plan_writes_parts_then_the_files_the_launcher_reads() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("index.json"), b"{}").unwrap();
    std::fs::write(root.path().join("oci-layout"), b"").unwrap();
    let files = [PathBuf::from("index.json"), PathBuf::from("oci-layout")];
    let env = [("LANG".to_string(), "C".to_string())].into();
    let plan = stage_plan(root.path(), &files, &["serve".to_string()], &env).unwrap();
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
        serde_json::json!({"args": ["serve"], "env": {"LANG": "C"}, "workspace": super::super::CONTAINER_WORKSPACE})
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
}
