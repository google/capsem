use super::*;

#[test]
fn short_path_uses_run_dir() {
    let run_dir = PathBuf::from("/tmp/r");
    let p = instance_socket_path(&run_dir, "vm-1");
    assert_eq!(p, PathBuf::from("/tmp/r/instances/vm-1.sock"));
}

#[test]
fn long_path_falls_back_to_tmp_capsem() {
    let run_dir = PathBuf::from(
        "/var/folders/lv/deeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeep/T/capsem-test-xxxx",
    );
    let p = instance_socket_path(&run_dir, "tmp-long-name-that-blows-past-sun-len");
    assert!(
        p.starts_with("/tmp/capsem/"),
        "expected fallback under /tmp/capsem/, got {}",
        p.display()
    );
    assert!(p.as_os_str().len() < SUN_PATH_MAX);
}
