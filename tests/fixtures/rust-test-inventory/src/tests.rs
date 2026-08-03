use super::*;

#[test]
fn library_sentinel() {
    assert_eq!(answer(), 42);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_sentinel() {
    assert_eq!(std::env::consts::OS, "macos");
}

#[cfg(target_os = "linux")]
#[test]
fn linux_sentinel() {
    assert_eq!(std::env::consts::OS, "linux");
}
